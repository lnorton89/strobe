// ============================================================================
// Module:       strobe_app::ui::browser
// Description:  The scene browser: the library's scene folders as a list,
//               the playing one highlighted.
//
// Dependencies: egui; crate::ui::model; crate::theme
// ============================================================================

//! The scene browser (left panel).

use super::model::{Action, Model};
use crate::theme;

/// Draws the scene list. A click asks the app to load that scene.
pub fn show(ui: &mut egui::Ui, model: &Model<'_>, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.heading("Scenes");
    ui.label(
        egui::RichText::new(model.library.dir.display().to_string())
            .small()
            .color(theme::muted()),
    );
    ui.separator();
    if model.library.entries.is_empty() {
        ui.colored_label(theme::muted(), "No scenes found in this folder.");
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, entry) in model.library.entries.iter().enumerate() {
                let playing = model.playing == Some(index);
                let label = egui::RichText::new(&entry.name);
                let label = if playing {
                    label.color(theme::accent()).strong()
                } else {
                    label
                };
                let response = ui.selectable_label(playing, label);
                if response.clicked() {
                    actions.push(Action::Load(index));
                }
            }
        });
}
