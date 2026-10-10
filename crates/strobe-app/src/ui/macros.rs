// ============================================================================
// Module:       strobe_app::ui::macros
// Description:  The Performance Macros strip: the playing scene's first few
//               controls, always at hand under the preview.
//
// Dependencies: egui; strobe-scene (ControlBank); super::{controls, theme,
//               widgets}, crate::workspace
// ============================================================================

//! Performance Macros, stage 1 (UI_DESIGN section 8).
//!
//! A macro is meant to be a persistent, performance-facing binding to a
//! target (a scene control, a meta control, a playlist action...) with a
//! source badge, a range override and an identity colour, that survives a
//! scene change by going *unavailable* rather than disappearing.
//!
//! **Stage 1 is deliberately smaller, and says so.** There is no curated set
//! yet: the strip shows the playing scene's first controls, in the order the
//! scene declares them, each marked `SCENE`. They are the same controls the
//! inspector edits (the bank is the one model, there is no second copy), so
//! moving one here moves it there. What is missing, and left to stage 2:
//!
//! - pinning, unpinning and reordering a persistent macro set;
//! - target resolution by exact id, group + name, alias, meta target;
//! - the `Unavailable`, `Broken`, `Disabled`, `Learning` and `Conflict`
//!   states (a stage-1 macro is always `Resolved`: it *is* the control);
//! - MIDI/OSC sources, shortcuts and identity colours.
//!
//! The strip is the one place the whole control set is not shown: it is
//! bounded to [`MAX_MACROS`] and to what fits, and a control whose widget
//! needs room (an XY pad) is replaced by a button that opens the inspector.

use super::controls;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::palette::Palette;
use crate::workspace::{Pane, Panes};
use egui::{CornerRadius, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use strobe_scene::control::{ControlBank, ControlKind};

/// The most macros the strip shows.
pub const MAX_MACROS: usize = 6;

/// The width of one macro cell, in points.
const CELL_WIDTH: f32 = 168.0;

/// The height of the strip.
pub const STRIP_HEIGHT: f32 = 50.0;

/// How many cells fit in `width`, between 1 and [`MAX_MACROS`].
pub fn cells_that_fit(width: f32, gap: f32) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    (((width + gap) / (CELL_WIDTH + gap)).floor() as usize).clamp(1, MAX_MACROS)
}

/// The first `count` controls of the bank, in declaration order.
pub fn pick(bank: &ControlBank, count: usize) -> Vec<usize> {
    (0..bank.defs().len()).take(count).collect()
}

/// Draws the strip. `bank` is `None` when no scene is playing.
pub fn strip(ui: &mut Ui, theme: &Palette, bank: Option<&mut ControlBank>, panes: &mut Panes) {
    let gap = ui.spacing().item_spacing.x;
    let count = cells_that_fit(ui.available_width(), gap);
    let Some(bank) = bank else {
        placeholder(
            ui,
            theme,
            "Macros appear here: the first controls of the playing scene.",
        );
        return;
    };
    let picked = pick(bank, count);
    if picked.is_empty() {
        placeholder(ui, theme, "This scene has no controls to pin.");
        return;
    }
    ui.horizontal(|ui| {
        for index in picked {
            cell(ui, theme, bank, index, panes);
        }
    });
}

fn placeholder(ui: &mut Ui, theme: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), STRIP_HEIGHT),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::TextStyle::Small.resolve(ui.style()),
        theme::rgb(theme.text_faint),
    );
}

/// One macro: a source badge, the control's name, and a widget sized for the
/// strip.
fn cell(ui: &mut Ui, theme: &Palette, bank: &mut ControlBank, index: usize, panes: &mut Panes) {
    let Some(def) = bank.defs().get(index).cloned() else {
        return;
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CELL_WIDTH, STRIP_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.raised),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
    let inner = rect.shrink2(Vec2::new(SPACE_SM, SPACE_XS + 1.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(SPACE_XS, 2.0);
            let target = bank.target(index).copied().unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("SCENE")
                        .size(9.0)
                        .strong()
                        .color(theme::rgb(theme.accent)),
                );
                let name = widgets::truncated(
                    ui,
                    &def.name,
                    egui::TextStyle::Small.resolve(ui.style()),
                    theme.text,
                    inner.width() - 80.0,
                );
                let (r, response) = ui.allocate_exact_size(name.size(), Sense::hover());
                ui.painter().galley(r.min, name, theme::rgb(theme.text));
                if let Some(text) = &def.description {
                    response.on_hover_text(text);
                }
                if let ControlKind::Slider(_) | ControlKind::Knob(_) = def.kind {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let places = controls::decimals(def.min[0], def.max[0]);
                        ui.label(
                            egui::RichText::new(format!("{:.places$}", target[0]))
                                .monospace()
                                .size(11.0)
                                .color(theme::rgb(theme.text_muted)),
                        );
                    });
                }
            });
            match def.kind {
                ControlKind::Slider(_) | ControlKind::Knob(_) => {
                    let mut value = if target[0].is_finite() {
                        target[0]
                    } else {
                        def.default[0]
                    };
                    let width = ui.available_width();
                    let (_, out) = controls::rail(
                        ui,
                        theme,
                        ("macro", index),
                        width,
                        &mut value,
                        def.min[0],
                        def.max[0],
                        def.default[0],
                    );
                    if out.reset {
                        bank.set_default(index);
                    } else if out.changed {
                        bank.set(index, Some(0), &[value]);
                    }
                }
                ControlKind::Toggle { .. } => {
                    let on = def.normalize(0, target[0]) >= 0.5;
                    let label = if on { "ON" } else { "OFF" };
                    if widgets::tool_button(
                        ui,
                        theme,
                        crate::icon::Icon::Check,
                        label,
                        on,
                        "Toggle",
                    )
                    .clicked()
                    {
                        let next = if on { def.min[0] } else { def.max[0] };
                        bank.set(index, Some(0), &[next]);
                    }
                }
                ControlKind::Bang(_) => {
                    if controls::bang_button(ui, theme, "Trigger").clicked() {
                        bank.press(index);
                    }
                }
                ControlKind::Dropdown { .. } => {
                    let selected = (target[0].round().max(0.0) as usize)
                        .min(def.labels.len().saturating_sub(1));
                    let current = def.labels.get(selected).cloned().unwrap_or_default();
                    egui::ComboBox::from_id_salt(("macro-dropdown", index))
                        .width(inner.width() - 4.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (option, label) in def.labels.iter().enumerate() {
                                if ui.selectable_label(option == selected, label).clicked() {
                                    bank.set(index, Some(0), &[option as f32]);
                                }
                            }
                        });
                }
                ControlKind::Color { .. } => {
                    let mut rgb = target;
                    if ui.color_edit_button_rgb(&mut rgb).changed() {
                        bank.set(index, None, &rgb);
                    }
                }
                ControlKind::Xy(_) => {
                    if widgets::tool_button(
                        ui,
                        theme,
                        crate::icon::Icon::Controls,
                        "XY pad",
                        false,
                        "Open the controls pane: an XY pad needs more room than a macro cell",
                    )
                    .clicked()
                    {
                        panes.open(Pane::Inspector);
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use strobe_scene::scene::SceneDef;

    fn bank(count: usize) -> Result<ControlBank> {
        let controls: Vec<String> = (0..count)
            .map(|i| format!(r#"{{ "NAME": "c{i}", "TYPE": "slider" }}"#))
            .collect();
        let json = format!(r#"{{ "CONTROLS": [{}] }}"#, controls.join(","));
        Ok(ControlBank::new(SceneDef::parse(&json, "t")?.controls)?)
    }

    #[test]
    fn the_strip_shows_the_first_controls_in_declaration_order() -> Result<()> {
        let bank = bank(10)?;
        assert_eq!(pick(&bank, 4), vec![0, 1, 2, 3], "first four");
        assert_eq!(
            pick(&bank, 99),
            (0..10).collect::<Vec<_>>(),
            "all there are"
        );
        assert!(pick(&bank, 0).is_empty(), "none asked for");
        Ok(())
    }

    #[test]
    fn a_wider_window_shows_more_macros_up_to_the_cap() {
        let gap = 8.0;
        let at = |w: f32| cells_that_fit(w, gap);
        assert_eq!(at(0.0), 1, "never zero cells");
        assert_eq!(at(f32::NAN), 1, "nonsense width");
        assert!(at(500.0) < at(900.0), "more room, more macros");
        assert_eq!(at(10_000.0), MAX_MACROS, "capped");
        assert!(
            at(1280.0 - 60.0 - 300.0) >= 4,
            "1280 with the inspector open still shows four macros"
        );
    }
}
