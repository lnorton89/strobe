// ============================================================================
// Module:       strobe_app::ui::command_palette
// Description:  The Ctrl+K palette: a search field over every command and
//               pane, drawn over a scrim, run with the keyboard or a click.
//
// Dependencies: egui; crate::{command_palette, palette, workspace};
//               super::{keys, model, theme, motion}
// ============================================================================

//! The command palette, drawn.
//!
//! [`crate::command_palette`] decides what is listed and in what order; this
//! module only draws it and reads the keys. Up and Down move the highlight,
//! Enter runs it, Escape or a click outside closes the palette, and Ctrl+K
//! toggles it. Running an entry goes through [`super::keys::run`], the same
//! code its shortcut runs, so the two can never disagree.
//!
//! While the palette is open it owns the keyboard, as the modal does: the
//! shell passes it to [`super::keys::handle`] as a modal, so typing `b` into
//! the field never also blacks out the show.

use super::keys;
use super::model::{Action, Model};
use super::motion;
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::command_palette::{Availability, Item, Target, items, moved, rank};
use crate::palette::Palette;
use egui::{Align2, Context, CornerRadius, Key, Modifiers, Sense, Vec2};

/// The card's width: room for a command name, its group and its shortcut on
/// one line.
const CARD_WIDTH: f32 = 520.0;

/// How far below the top of the window the card sits, so it reads as
/// attached to the title bar rather than floating mid-screen.
const TOP_OFFSET: f32 = 72.0;

/// The most rows shown at once; the list scrolls with the highlight.
const MAX_ROWS: usize = 10;

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
        }
    }

    /// Opens it with `query` already typed.
    pub fn open_with(&mut self, query: &str) {
        self.open = true;
        query.clone_into(&mut self.query);
        self.selected = Some(0);
        self.just_opened = true;
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
    let all = items(model.remote.bindings, availability);
    let just_opened = std::mem::take(&mut model.ui.command_palette.just_opened);

    // Consumed before the field is drawn, so it never sees Enter (which
    // would end the edit) or the arrows.
    let keys = ctx.input_mut(|input| Keys {
        up: input.consume_key(Modifiers::NONE, Key::ArrowUp),
        down: input.consume_key(Modifiers::NONE, Key::ArrowDown),
        enter: input.consume_key(Modifiers::NONE, Key::Enter),
        close: input.consume_key(Modifiers::NONE, Key::Escape)
            || (!just_opened && input.consume_key(Modifiers::COMMAND, Key::K)),
    });

    let state = &mut model.ui.command_palette;
    let mut ranked = rank(&all, &state.query);
    if keys.up {
        state.selected = moved(state.selected, -1, ranked.len());
    }
    if keys.down {
        state.selected = moved(state.selected, 1, ranked.len());
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
                    ui.set_width(CARD_WIDTH);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .id(egui::Id::new("palette-query"))
                            .hint_text("Type a command or a pane")
                            .desired_width(f32::INFINITY),
                    );
                    if just_opened {
                        field.request_focus();
                    }
                    if field.changed() {
                        ranked = rank(&all, &state.query);
                        state.selected = (!ranked.is_empty()).then_some(0);
                    }
                    ui.add_space(SPACE_SM);
                    if let Some(row) = rows(ui, theme, &all, &ranked, state.selected, &state.query)
                    {
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
        .and_then(|row| ranked.get(row))
        .and_then(|index| all.get(*index))
        .filter(|item| item.unavailable.is_none())
        .map(|item| item.target)
    else {
        return;
    };
    state.close();
    match target {
        Target::Command(command) => keys::run(command, model, actions),
        Target::Pane(pane) => model.ui.panes.open(pane),
    }
}

/// Draws the visible rows. Returns the row clicked, if one was.
fn rows(
    ui: &mut egui::Ui,
    theme: &Palette,
    all: &[Item],
    ranked: &[usize],
    selected: Option<usize>,
    query: &str,
) -> Option<usize> {
    if ranked.is_empty() {
        ui.label(
            egui::RichText::new(format!(
                "No command or pane matches \u{201c}{}\u{201d}",
                query.trim()
            ))
            .color(theme::rgb(theme.text_muted)),
        );
        return None;
    }
    // A window of rows that keeps the highlight in view.
    let highlight = selected.unwrap_or(0);
    let first = highlight
        .saturating_sub(MAX_ROWS - 1)
        .min(ranked.len().saturating_sub(MAX_ROWS));
    let mut clicked = None;
    for (row, index) in ranked.iter().enumerate().skip(first).take(MAX_ROWS) {
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
        let left = rect.left_center() + Vec2::new(SPACE_SM, 0.0);
        let title = ui.painter().text(
            left,
            Align2::LEFT_CENTER,
            &item.title,
            body,
            theme::rgb(text),
        );
        ui.painter().text(
            title.right_center() + Vec2::new(SPACE_SM, 0.0),
            Align2::LEFT_CENTER,
            item.category,
            small.clone(),
            theme::rgb(theme.text_faint),
        );
        let right = item
            .unavailable
            .or(item.shortcut.as_deref())
            .unwrap_or_default();
        ui.painter().text(
            rect.right_center() - Vec2::new(SPACE_SM, 0.0),
            Align2::RIGHT_CENTER,
            right,
            small,
            theme::rgb(theme.text_muted),
        );
        if response.clicked() {
            clicked = Some(row);
        }
    }
    clicked
}
