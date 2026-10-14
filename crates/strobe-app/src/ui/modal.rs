// ============================================================================
// Module:       strobe_app::ui::modal
// Description:  The one confirmation card, and the scrim behind it.
//
// Dependencies: egui; super::{theme, widgets, motion}, crate::palette.
//               Ported from rustaman's `gui::ui::modal` (D5, D130).
// ============================================================================

//! Confirmations.
//!
//! **There is one modal, not several.** A single card asks whichever
//! question is pending, and there is no `egui::Window` anywhere in this app:
//! a window per question is how an app ends up with six dialogs of six
//! widths, three of them movable for no reason, and one that can be left
//! open behind the main window.
//!
//! Strobe uses it sparingly. UI_IMPLEMENTATION_PLAN section 14 says to avoid
//! modal error dialogs during performance, and routine actions (loading a
//! scene, blackout, moving a slider) never ask. The only question today is
//! the one that cannot be a banner because it changes files: importing a
//! shader that is already in the library (D113).
//!
//! ## What a confirmation has to say
//!
//! Three things: **what will happen** (in the words of the action, not "Are
//! you sure?"), **what it will happen to** (named the way the list named it),
//! and **what it costs** when that is not obvious. The confirming button
//! says what it does, never "OK".
//!
//! ## Escape cancels, Enter does not confirm a destructive action
//!
//! A dialog that can only be dismissed with the mouse interrupts typing, so
//! Escape cancels. Enter confirms only where the action is not destructive;
//! binding it to a destructive button means a stray keystroke while the card
//! is opening does the thing.

use super::motion;
use super::theme::{self, SPACE_LG, SPACE_SM};
use super::widgets;
use crate::palette::Palette;
use egui::{Align2, CornerRadius, Key, Sense, Ui, Vec2};

/// The modal card's width: wide enough for a sentence naming a scene without
/// wrapping mid-name, narrow enough to read as a dialog rather than a page.
const CARD_WIDTH: f32 = 420.0;

/// The scrim's opacity over the window behind, enough to push it back
/// without hiding it.
const SCRIM_ALPHA: u8 = 160;

/// How far the card rises as it arrives.
const RISE: f32 = 6.0;

/// What a pending confirmation asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    /// The heading: what will happen, and to what.
    pub title: String,
    /// The body: what it costs.
    pub body: String,
    /// The confirming button's label.
    pub confirm: String,
    /// The cancelling button's label.
    pub cancel: String,
    /// Whether the confirming action is destructive, which decides the
    /// button's colour and whether Enter confirms.
    pub destructive: bool,
}

impl Confirmation {
    /// Whether Enter should confirm rather than cancel: never for a
    /// destructive action.
    #[must_use]
    pub fn default_is_confirm(&self) -> bool {
        !self.destructive
    }
}

/// Draws the card for `question` over a scrim. Returns the answer once given:
/// `Some(true)` to confirm, `Some(false)` to cancel.
pub fn draw(ctx: &egui::Context, theme: &Palette, question: &Confirmation) -> Option<bool> {
    let progress = motion::transition(ctx, egui::Id::new("modal-card"), true, motion::QUICK);
    let screen = ctx.content_rect();
    // Painted on a foreground layer so it covers every panel.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("modal"),
    ));
    let scrim_alpha = (f32::from(SCRIM_ALPHA) * progress) as u8;
    painter.rect_filled(
        screen,
        CornerRadius::ZERO,
        theme::translucent(theme.app, scrim_alpha),
    );

    let mut answer = None;
    // An `Area` rather than a `Window`: no title bar, no move handle, no
    // close button, and it cannot be left open behind the main window.
    egui::Area::new(egui::Id::new("modal-card"))
        .order(egui::Order::Foreground)
        .anchor(
            Align2::CENTER_CENTER,
            Vec2::new(0.0, RISE * (1.0 - progress)),
        )
        .show(ctx, |ui| {
            ui.set_opacity(progress);
            egui::Frame::new()
                .fill(theme::rgb(theme.raised))
                .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(theme::margin(SPACE_LG))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    // Alpha-only: a shadow is lighting, not a theme colour.
                    color: egui::Color32::from_black_alpha(96),
                })
                .show(ui, |ui| {
                    buttons_and_text(ui, theme, question, &mut answer);
                });
        });

    ctx.input(|input| {
        if input.key_pressed(Key::Escape) {
            answer = Some(false);
        }
        if input.key_pressed(Key::Enter) && question.default_is_confirm() {
            answer = Some(true);
        }
    });

    // The scrim swallows clicks so a click that misses the card does not
    // reach the pane behind it.
    egui::Area::new(egui::Id::new("modal-scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            // The scrim only has to take the clicks; nothing reads them.
            let _ = ui.allocate_exact_size(screen.size(), Sense::click());
        });
    answer
}

fn buttons_and_text(
    ui: &mut Ui,
    theme: &Palette,
    question: &Confirmation,
    answer: &mut Option<bool>,
) {
    ui.set_width(CARD_WIDTH);
    ui.label(
        egui::RichText::new(&question.title)
            .color(theme::rgb(theme.text))
            .size(16.0)
            .strong(),
    );
    ui.add_space(SPACE_SM);
    ui.label(
        egui::RichText::new(&question.body)
            .color(theme::rgb(theme.text_muted))
            .text_style(egui::TextStyle::Small),
    );
    ui.add_space(SPACE_LG);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // The destructive button is on the right, where the eye ends up, but
        // Cancel is adjacent to it rather than across the card: a gap
        // between them is a gap the pointer travels through on the way to
        // the wrong one.
        let confirmed = if question.destructive {
            danger_button(ui, theme, &question.confirm)
        } else {
            widgets::primary_button(ui, theme, &question.confirm)
        };
        if confirmed.clicked() {
            *answer = Some(true);
        }
        ui.add_space(SPACE_SM);
        if widgets::button(ui, theme, None, &question.cancel).clicked() {
            *answer = Some(false);
        }
    });
}

/// A destructive button: the danger colour, with the on-accent label.
fn danger_button(ui: &mut Ui, theme: &Palette, label: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), font, theme::rgb(theme.text_on_accent));
    let size = galley.size() + Vec2::new(theme::SPACE_MD * 2.0, SPACE_SM);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = widgets::hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.danger,
        theme.danger,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(theme.text_on_accent),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace() -> Confirmation {
        Confirmation {
            title: "Replace \"fractal-tunnel\"?".to_string(),
            body: "This shader is already in the library.".to_string(),
            confirm: "Replace scene".to_string(),
            cancel: "Keep existing".to_string(),
            destructive: true,
        }
    }

    #[test]
    fn enter_never_confirms_a_destructive_action() {
        // A stray keystroke while the card is opening must not do the thing.
        assert!(!replace().default_is_confirm());
        let harmless = Confirmation {
            destructive: false,
            ..replace()
        };
        assert!(harmless.default_is_confirm(), "a harmless question may");
    }

    #[test]
    fn the_confirming_button_says_what_it_does() {
        let label = replace().confirm.to_lowercase();
        assert!(
            label != "ok" && label != "yes",
            "the button should name the action, got {label}"
        );
    }

    #[test]
    fn the_question_names_what_it_acts_on() {
        assert!(replace().title.contains("fractal-tunnel"));
    }

    /// Runs the card for one frame with `events` and returns its answer.
    fn answer_with(events: Vec<egui::Event>) -> Option<bool> {
        let ctx = egui::Context::default();
        let theme = crate::palette::fallback().clone();
        theme::apply(&ctx, &theme, theme::Density::default());
        let question = replace();
        let mut answer = None;
        let _ = crate::ui::testing::run(
            &ctx,
            crate::ui::testing::input(Vec2::new(800.0, 600.0), events),
            |ui| {
                answer = draw(ui.ctx(), &theme, &question);
            },
        );
        answer
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn escape_cancels_and_enter_does_not_confirm_a_destructive_question() {
        assert_eq!(answer_with(vec![key(Key::Escape)]), Some(false), "escape");
        assert_eq!(answer_with(vec![key(Key::Enter)]), None, "enter");
        assert_eq!(answer_with(Vec::new()), None, "no key, no answer");
    }
}
