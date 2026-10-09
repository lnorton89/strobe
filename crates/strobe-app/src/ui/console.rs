// ============================================================================
// Module:       strobe_app::ui::console
// Description:  The bottom panel: the frame-time readout and the console of
//               load and compile messages.
//
// Dependencies: egui; crate::ui::model; crate::console; crate::theme
// ============================================================================

//! The status strip and console (bottom panel).

use super::model::{Action, Model};
use crate::console::Level;
use crate::theme;

/// Draws the stats line, then the console below it.
pub fn show(ui: &mut egui::Ui, model: &Model<'_>, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "CPU {:.2} ms   {:.1} fps",
                model.stats.cpu_ms(),
                model.stats.fps()
            ))
            .monospace()
            .color(theme::accent()),
        );
        ui.label(
            egui::RichText::new(format!("audio {}", model.audio.source))
                .monospace()
                .color(if model.audio.live {
                    theme::accent()
                } else {
                    theme::muted()
                }),
        );
        if let Some(preview) = model.preview {
            ui.label(
                egui::RichText::new(format!("render {}x{}", preview.size[0], preview.size[1]))
                    .monospace()
                    .color(theme::muted()),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Clear").clicked() {
                actions.push(Action::ClearConsole);
            }
            ui.label(egui::RichText::new("Console").color(theme::muted()));
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in model.console.lines() {
                let text = egui::RichText::new(&line.text).monospace();
                match line.level {
                    Level::Info => ui.label(text),
                    Level::Debug => ui.label(text.color(theme::muted())),
                    Level::Warn => ui.label(text.color(theme::warning())),
                    Level::Error => ui.label(text.color(theme::error())),
                };
            }
        });
}
