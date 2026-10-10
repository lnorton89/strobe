// ============================================================================
// Module:       strobe_app::ui::transport
// Description:  The strip under the preview: previous, hold, next, the cue
//               state, the tempo and the blackout, then the macros.
//
// Dependencies: egui; super::{model, theme, widgets, macros, icon},
//               crate::icon
// ============================================================================

//! The transport and cue strip (UI_IMPLEMENTATION_PLAN section 13).
//!
//! ```text
//! [PREV] [HOLD] [NEXT]   NOW <scene>   NEXT <scene>   00:41   128 BPM   [BLACKOUT]
//! [ macro ][ macro ][ macro ][ macro ][ macro ][ macro ]
//! ```
//!
//! The strip answers: what is happening, what happens next, how long it has
//! been, and what immediate action is available. It does not edit a
//! playlist.
//!
//! ## What is wired, and what is not
//!
//! Stage 1 has no cue list in the app (the playlist crate is not a
//! dependency yet), so the strip is honest about it rather than faking one:
//!
//! - **Prev / Next** step through the library in its sorted order, skipping
//!   scenes known to be broken. "NEXT" on the strip names what a press would
//!   load, and the strip says `LIBRARY ORDER` beside it.
//! - **Hold** is drawn as unavailable, with a tooltip that says why. Space
//!   is reserved for it; pressing Space shows a notice instead of doing
//!   nothing silently.
//! - **Time** is the playing scene's clock, not a cue's remaining time, and
//!   is labelled as such.
//! - **Blackout** is real: the shown output goes black, the preview keeps
//!   showing the scene (the operator must still see it), and the scene
//!   clock keeps running (OPEN_QUESTIONS: whether blackout should pause it).
//!   It is outlined in the danger colour, fills when on, and says
//!   `BLACKOUT ON` in words.

use super::icon as icons;
use super::macros;
use super::model::{Action, Model};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets::{self, Look};
use crate::icon::Icon;
use crate::palette::Palette;
use egui::{Align, Layout, Sense, Ui, Vec2};

/// The height of the whole strip: the transport row, a gap, and the macros.
pub const HEIGHT: f32 = theme::TRANSPORT_BUTTON + macros::STRIP_HEIGHT + 3.0 * SPACE_SM;

// The strip must hold both rows; checked when the crate is compiled.
const _: () = assert!(
    HEIGHT >= theme::TRANSPORT_BUTTON + macros::STRIP_HEIGHT,
    "the transport strip is shorter than its two rows"
);

/// Below this width the strip drops the NEXT name and keeps the essentials:
/// previous, next, the scene, blackout (UI_RESEARCH_ADDENDUM section 9).
const NARROW: f32 = 900.0;

/// `mm:ss` for a number of seconds; `--:--` for nonsense.
pub fn clock(seconds: f32) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "--:--".to_string();
    }
    let total = seconds as u64;
    format!("{:02}:{:02}", (total / 60) % 100, total % 60)
}

/// Draws the strip.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    egui::Panel::bottom("transport")
        .exact_size(HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
                .inner_margin(theme::margin_xy(SPACE_MD, SPACE_SM)),
        )
        .show(ui, |ui| {
            let narrow = ui.ctx().content_rect().width() < NARROW;
            transport_row(ui, model, theme, narrow, actions);
            ui.add_space(SPACE_SM);
            macros::strip(
                ui,
                theme,
                model.controls.as_deref_mut(),
                &mut model.ui.panes,
            );
        });
}

fn transport_row(
    ui: &mut Ui,
    model: &Model<'_>,
    theme: &Palette,
    narrow: bool,
    actions: &mut Vec<Action>,
) {
    let t = model.transport;
    ui.horizontal(|ui| {
        ui.set_height(theme::TRANSPORT_BUTTON);

        let step = if t.can_step {
            Look::Normal
        } else {
            Look::Unavailable
        };
        let step_tip = |name: &str, key: &str| {
            if t.can_step {
                format!("{name} scene in the library ({key})")
            } else {
                "There is only one scene in the library".to_string()
            }
        };
        if widgets::transport_button(ui, theme, Icon::Prev, step, &step_tip("Previous", "Left"))
            .clicked()
        {
            actions.push(Action::PrevScene);
        }
        // Hold: not wired to anything. Drawn as absent, and says why.
        widgets::transport_button(
            ui,
            theme,
            Icon::Pause,
            Look::Unavailable,
            "Hold: not available yet. There is no cue list to hold until the \
             Playlist workspace lands (stage 2). Space is reserved for it.",
        );
        if widgets::transport_button(ui, theme, Icon::Next, step, &step_tip("Next", "Right"))
            .clicked()
        {
            actions.push(Action::NextScene);
        }
        ui.add_space(SPACE_SM);

        // Right side first, so the cue text gets what is left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let tip = if t.blackout {
                "Bring the output back (B)"
            } else {
                "Black out the output (B). The preview keeps showing the scene."
            };
            if widgets::blackout_button(ui, theme, t.blackout, tip).clicked() {
                actions.push(Action::SetBlackout(!t.blackout));
            }
            ui.add_space(SPACE_SM);
            let bpm = if model.audio.bpm > 0.0 {
                format!("{:.0} BPM", model.audio.bpm)
            } else {
                "-- BPM".to_string()
            };
            ui.label(
                egui::RichText::new(bpm)
                    .monospace()
                    .size(15.0)
                    .color(theme::rgb(theme.text)),
            );
            ui.add_space(SPACE_SM);
            ui.label(
                egui::RichText::new(clock(t.scene_time))
                    .monospace()
                    .size(15.0)
                    .color(theme::rgb(theme.text)),
            )
            .on_hover_text("Time since the scene started. (There are no cues yet, so this is not a cue's remaining time.)");
            ui.add_space(SPACE_SM);
            cue_text(ui, model, theme, narrow);
        });
    });
}

/// NOW and NEXT, truncated to the room that is left.
fn cue_text(ui: &mut Ui, model: &Model<'_>, theme: &Palette, narrow: bool) {
    let t = model.transport;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width.max(0.0), theme::TRANSPORT_BUTTON),
        Sense::hover(),
    );
    let painter = ui.painter();
    let label_font = egui::FontId::new(10.0, egui::FontFamily::Proportional);
    let name_font = egui::FontId::new(15.0, egui::FontFamily::Proportional);

    // NOW: a small label over the scene name, with its identity swatch.
    let swatch = rect.left_center() + Vec2::new(4.0, 0.0);
    match t.now {
        Some(name) => {
            painter.circle_filled(swatch, 4.0, theme::rgb(widgets::scene_colour(theme, name)));
        }
        None => {
            painter.circle_stroke(
                swatch,
                4.0,
                egui::Stroke::new(1.0, theme::rgb(theme.text_faint)),
            );
        }
    }
    let x = rect.left() + SPACE_MD + SPACE_XS;
    let half = if narrow || t.next.is_none() {
        rect.width()
    } else {
        rect.width() * 0.58
    };
    painter.text(
        egui::pos2(x, rect.top() + 6.0),
        egui::Align2::LEFT_CENTER,
        "NOW",
        label_font.clone(),
        theme::rgb(theme.text_muted),
    );
    let now = t.now.unwrap_or("No scene playing");
    let colour = if t.now.is_some() {
        theme.text
    } else {
        theme.text_faint
    };
    let galley = widgets::truncated(
        ui,
        now,
        name_font.clone(),
        colour,
        half - SPACE_MD - SPACE_LG_AND_A_BIT,
    );
    ui.painter().galley(
        egui::pos2(x, rect.bottom() - galley.size().y - 1.0),
        galley,
        theme::rgb(colour),
    );

    if !narrow && let Some(next) = t.next {
        let nx = rect.left() + half;
        let painter = ui.painter();
        painter.text(
            egui::pos2(nx, rect.top() + 6.0),
            egui::Align2::LEFT_CENTER,
            "NEXT  (library order)",
            label_font,
            theme::rgb(theme.text_muted),
        );
        let galley = widgets::truncated(
            ui,
            next,
            name_font,
            theme.text_muted,
            rect.right() - nx - SPACE_SM,
        );
        ui.painter().galley(
            egui::pos2(nx, rect.bottom() - galley.size().y - 1.0),
            galley,
            theme::rgb(theme.text_muted),
        );
    }
    let _ = icons::INLINE;
    response.on_hover_text(
        "Scenes step in library order. A cue list with real NEXT and remaining time \
         arrives with the Playlist workspace (stage 2).",
    );
}

/// Room the NOW column keeps free of its swatch and padding.
const SPACE_LG_AND_A_BIT: f32 = theme::SPACE_LG + SPACE_XS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(41.9), "00:41");
        assert_eq!(clock(70.0), "01:10");
        assert_eq!(clock(3599.0), "59:59");
    }

    #[test]
    fn the_clock_survives_nonsense() {
        assert_eq!(clock(f32::NAN), "--:--");
        assert_eq!(clock(-3.0), "--:--");
        assert_eq!(clock(f32::INFINITY), "--:--");
        assert_eq!(clock(1.0e9).len(), 5, "a very long run still fits");
    }
}
