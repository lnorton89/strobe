// ============================================================================
// Module:       strobe_app::ui::layout
// Description:  Arranges the control window: output strip on top, scene
//               browser left, controls right, console below, preview centre.
//
// Dependencies: egui; crate::ui::{browser, controls, console, model};
//               crate::{session, theme}
// ============================================================================

//! The control window's layout.
//!
//! Minimal on purpose. The Perform-first workspace shell in
//! `docs/UI_DESIGN.md` replaces this; what carries over is the contract:
//! panels read a [`Model`] and return [`Action`]s.

use super::model::{Action, Model};
use super::{browser, console, controls};
use crate::session::RENDER_SCALE;
use crate::theme;

/// Width of the scene browser, in points.
const BROWSER_WIDTH: f32 = 220.0;
/// Width of the controls panel, in points.
const CONTROLS_WIDTH: f32 = 340.0;
/// Height of the console panel, in points.
const CONSOLE_HEIGHT: f32 = 170.0;

/// Draws the whole control window.
pub fn show(ui: &mut egui::Ui, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    egui::Panel::top("output-strip").show(ui, |ui| output_strip(ui, model, actions));
    egui::Panel::bottom("console")
        .resizable(true)
        .default_size(CONSOLE_HEIGHT)
        .show(ui, |ui| console::show(ui, model, actions));
    egui::Panel::left("browser")
        .resizable(true)
        .default_size(BROWSER_WIDTH)
        .show(ui, |ui| browser::show(ui, model, actions));
    egui::Panel::right("controls")
        .resizable(true)
        .default_size(CONTROLS_WIDTH)
        .show(ui, |ui| controls_panel(ui, model, actions));
    egui::CentralPanel::default().show(ui, |ui| preview(ui, model));
}

fn output_strip(ui: &mut egui::Ui, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new("STROBE")
                .strong()
                .color(theme::accent())
                .size(16.0),
        );
        ui.separator();
        let label = if model.fullscreen {
            "Exit fullscreen (F11)"
        } else {
            "Fullscreen (F11)"
        };
        if ui.button(label).clicked() {
            actions.push(Action::ToggleFullscreen);
        }
        monitor_picker(ui, model, actions);
        ui.separator();
        ui.label("Render scale");
        let slider = ui.add(
            egui::Slider::new(model.render_scale, RENDER_SCALE)
                .step_by(0.05)
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
        );
        if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
            actions.push(Action::SetRenderScale(*model.render_scale));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(model.gl_summary)
                    .small()
                    .color(theme::muted()),
            );
        });
    });
}

fn monitor_picker(ui: &mut egui::Ui, model: &Model<'_>, actions: &mut Vec<Action>) {
    let current = model
        .output_monitor
        .and_then(|index| model.monitors.get(index))
        .map_or("Output monitor", String::as_str);
    egui::ComboBox::from_id_salt("output-monitor")
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (index, label) in model.monitors.iter().enumerate() {
                if ui
                    .selectable_label(model.output_monitor == Some(index), label)
                    .clicked()
                {
                    actions.push(Action::MoveToMonitor(index));
                }
            }
        });
}

fn controls_panel(ui: &mut egui::Ui, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Controls");
        if model.controls.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Reload").clicked() {
                    actions.push(Action::Reload);
                }
            });
        }
    });
    ui.label(egui::RichText::new(model.scene_title).color(theme::muted()));
    ui.separator();
    match model.controls.as_deref_mut() {
        Some(bank) if !bank.defs().is_empty() => controls::show(ui, bank),
        Some(_) => {
            ui.colored_label(theme::muted(), "This scene has no controls.");
        }
        None => {
            ui.colored_label(theme::muted(), "Pick a scene on the left.");
        }
    }
}

/// The live preview, fitted to the panel and centred.
fn preview(ui: &mut egui::Ui, model: &Model<'_>) {
    let area = ui.available_rect_before_wrap();
    ui.painter()
        .rect_filled(area, 0.0, theme::preview_backdrop());
    let Some(preview) = model.preview else {
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            "No scene playing",
            egui::FontId::proportional(18.0),
            theme::muted(),
        );
        return;
    };
    let fit = fit_inside(area.size(), preview.aspect);
    let rect = egui::Rect::from_center_size(area.center(), fit);
    // The player's targets are bottom-up, egui's texture space is top-down.
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
    ui.painter()
        .image(preview.texture, rect, uv, egui::Color32::WHITE);
}

/// The largest rectangle of `aspect` (width over height) inside `bounds`.
fn fit_inside(bounds: egui::Vec2, aspect: f32) -> egui::Vec2 {
    if aspect <= 0.0 || bounds.x <= 0.0 || bounds.y <= 0.0 {
        return egui::Vec2::ZERO;
    }
    if bounds.x / bounds.y > aspect {
        egui::vec2(bounds.y * aspect, bounds.y)
    } else {
        egui::vec2(bounds.x, bounds.x / aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_in_a_tall_panel_fills_the_width() {
        let fit = fit_inside(egui::vec2(400.0, 800.0), 16.0 / 9.0);
        assert!((fit.x - 400.0).abs() < 0.01, "width {}", fit.x);
        assert!((fit.y - 225.0).abs() < 0.01, "height {}", fit.y);
    }

    #[test]
    fn a_wide_frame_in_a_wide_panel_fills_the_height() {
        let fit = fit_inside(egui::vec2(1600.0, 450.0), 16.0 / 9.0);
        assert!((fit.y - 450.0).abs() < 0.01, "height {}", fit.y);
        assert!((fit.x - 800.0).abs() < 0.01, "width {}", fit.x);
    }
}
