// ============================================================================
// Module:       strobe_app::ui::help
// Description:  The help bar along the window's bottom edge: one line that
//               says what the control under the pointer does, or, with the
//               pointer elsewhere, what the main view is for.
//
// Dependencies: egui; super::{theme, widgets}, crate::workspace
// ============================================================================

//! The help bar (UI_DESIGN section 4, D284).
//!
//! Synesthesia keeps a thin bar under everything that explains the control
//! the pointer is on. Strobe's controls already carry tooltips; the bar
//! shows the same sentence where a person is already looking, so a tooltip
//! never has to cover the thing it describes.
//!
//! ## How a widget reaches it
//!
//! A control that has a tooltip calls [`hovered`] with the response and the
//! words. While the pointer is on the control, the words are copied into a
//! buffer that lives as long as the window does. The bar is drawn before the
//! panels above it, so it shows what the *previous* frame's hover said: a
//! frame behind, which no one can see, and it keeps the bar from reordering
//! the layout. The buffers are reused, so the only allocation per frame is
//! the text's own layout, as for every label.
//!
//! ## What it does not do
//!
//! It never carries anything a person must read: a failure, a refused
//! write, a question. Those are banners and modals (D260, D285). It is the first
//! thing dropped when the window is short, so BLACKOUT and the transport
//! strip keep their room ([`MIN_WINDOW_HEIGHT`]).

use super::browser::LibraryPage;
use super::theme::{self, SPACE_SM};
use super::widgets;
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Response, Ui};
use std::cell::RefCell;

/// The bar's height.
pub const HEIGHT: f32 = 24.0;

/// Below this window height the bar is not drawn: the transport strip and
/// the macros need the room more.
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;

/// The two lines of text: what the pointer is on now, and what it was on
/// when the bar was last drawn.
#[derive(Default)]
struct Hints {
    /// Set by controls as they are hovered this frame.
    next: String,
    /// What the bar draws: the previous frame's `next`.
    shown: String,
}

thread_local! {
    static HINTS: RefCell<Hints> = RefCell::new(Hints::default());
}

/// Says that `response`'s control does `text`, while the pointer is on it,
/// and returns the response with `text` as its tooltip too.
pub fn hovered(response: Response, text: &str) -> Response {
    if response.hovered() {
        say(text);
    }
    response.on_hover_text(text)
}

/// Puts `text` on the bar for the next frame, unless a control already has
/// (the first to speak wins, and that is the innermost).
pub fn say(text: &str) {
    HINTS.with(|hints| {
        let mut hints = hints.borrow_mut();
        if hints.next.is_empty() {
            hints.next.push_str(text);
        }
    });
}

/// What the main view is for, for the bar when the pointer is on nothing in
/// particular. The library says which of its pages is up (D291).
fn idle(main: Option<Pane>, page: LibraryPage) -> &'static str {
    match main {
        None => "The live preview. Pick a view on the rail; Esc goes back one view.",
        Some(Pane::Browser) => match page {
            LibraryPage::Scenes => {
                "Click a scene to make it live; Ctrl+L flips to its controls. Ctrl+K opens every \
                 command."
            }
            LibraryPage::Controls => {
                "The live scene's controls. Ctrl+L flips back to the scenes; right-click a \
                 control's name to pin it as a macro."
            }
        },
        Some(Pane::Import) => "Paste a Shadertoy URL or id, or drop a file: it becomes a scene.",
        Some(Pane::Playlist) => {
            "The playlist: drag scenes in from the library, double-click a cue to jump to it."
        }
        Some(Pane::Editor) => {
            "The scene's files. Save writes them; Save and reload shows the result."
        }
        Some(Pane::Map) => {
            "Map a MIDI note or an OSC address to any control: press Learn, then move it."
        }
        Some(Pane::Diagnostics) => "What the machine and the show are doing, and the console.",
        Some(Pane::Settings) => "Output, audio, appearance, the LAN remote and shortcuts.",
        Some(Pane::Inspector) => "The panel: preview, controls, playlist, presets and edit.",
    }
}

/// Draws the bar for a main view of `main` (the library on `page`), and
/// starts the next frame's hover. Draws nothing in a window shorter than
/// [`MIN_WINDOW_HEIGHT`].
pub fn show(ui: &mut Ui, theme: &Palette, main: Option<Pane>, page: LibraryPage) {
    // The hover the controls set last frame becomes what is drawn, and the
    // buffer for this frame's is emptied, keeping its capacity.
    HINTS.with(|hints| {
        let mut hints = hints.borrow_mut();
        let hints = &mut *hints;
        std::mem::swap(&mut hints.shown, &mut hints.next);
        hints.next.clear();
    });
    if ui.ctx().content_rect().height() < MIN_WINDOW_HEIGHT {
        return;
    }
    egui::Panel::bottom("help-bar")
        .exact_size(HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_SM, 0.0)),
        )
        .show(ui, |ui| {
            let area = ui.max_rect();
            let font = egui::TextStyle::Small.resolve(ui.style());
            let (galley, colour) = HINTS.with(|hints| {
                let hints = hints.borrow();
                let (text, colour) = if hints.shown.is_empty() {
                    (idle(main, page), theme.text_faint)
                } else {
                    (hints.shown.as_str(), theme.text_muted)
                };
                (
                    widgets::truncated(ui, text, font, colour, area.width()),
                    colour,
                )
            });
            let pos = egui::pos2(area.left(), area.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, theme::rgb(colour));
        });
}
