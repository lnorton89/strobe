// ============================================================================
// Module:       strobe_app::ui::import
// Description:  The Shadertoy import panel: a path/id field, the result line,
//               the "replace?" question, and the browser-listener toggle with
//               its token.
//
// Dependencies: egui; crate::ui::model; crate::{import, theme}
// ============================================================================

//! The import panel (left column, under the scene list).
//!
//! It only edits the model's text field and pushes [`Action`]s; the app owns
//! the worker, the listener and the library. A drop anywhere on the window
//! is handled by the app, so this panel just says "drop here" while a file
//! is being dragged over.

use super::model::{Action, ImportPanel};
use crate::import::Status;
use crate::theme;

/// Draws the panel.
pub fn show(ui: &mut egui::Ui, panel: &mut ImportPanel<'_>, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.heading("Import from Shadertoy");
    if panel.drop_hover {
        ui.colored_label(theme::accent(), "Drop the .json file to import it");
    }
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(panel.text)
                .hint_text("file path, shader id or URL")
                .desired_width(ui.available_width() - 64.0),
        );
        let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = ui
            .add_enabled(!panel.text.trim().is_empty(), egui::Button::new("Import"))
            .on_hover_text(
                "An exported Shadertoy .json file (you can also drop one on this window), \
                 or a shader id/URL to fetch with your API key.",
            )
            .clicked();
        if (clicked || submitted) && !panel.text.trim().is_empty() {
            actions.push(Action::Import(panel.text.trim().to_string()));
        }
    });
    status_line(ui, panel.status, panel.busy);
    if let Some(title) = panel.replace_title {
        ui.horizontal(|ui| {
            if ui.button("Replace").clicked() {
                actions.push(Action::ConfirmReplace);
            }
            if ui.button("Cancel").clicked() {
                actions.push(Action::CancelReplace);
            }
            ui.label(
                egui::RichText::new(format!("Replace \"{title}\"?"))
                    .small()
                    .color(theme::warning()),
            );
        });
    }

    egui::CollapsingHeader::new("Browser userscript")
        .id_salt("import-listener")
        .default_open(panel.listener.enabled)
        .show(ui, |ui| listener(ui, panel, actions));
}

fn status_line(ui: &mut egui::Ui, status: &Status, busy: bool) {
    let (text, colour) = match status {
        Status::Idle => return,
        Status::Working(text) => (text.as_str(), theme::muted()),
        Status::Done(text) => (text.as_str(), theme::accent()),
        Status::Warned(text) => (text.as_str(), theme::warning()),
        Status::Failed(text) => (text.as_str(), theme::error()),
    };
    ui.horizontal_wrapped(|ui| {
        if busy {
            ui.spinner();
        }
        ui.label(egui::RichText::new(text).small().color(colour));
    });
}

fn listener(ui: &mut egui::Ui, panel: &ImportPanel<'_>, actions: &mut Vec<Action>) {
    let mut enabled = panel.listener.enabled;
    if ui
        .checkbox(&mut enabled, "Accept shaders from the browser")
        .on_hover_text(
            "Listens on 127.0.0.1 only. The \"Send to Strobe\" userscript on shadertoy.com \
             posts a shader here, with the token below.",
        )
        .changed()
    {
        actions.push(Action::SetImportListener(enabled));
    }
    ui.label(
        egui::RichText::new(panel.listener.state)
            .small()
            .color(if panel.listener.failed {
                theme::error()
            } else {
                theme::muted()
            }),
    );
    if let Some(token) = panel.listener.token {
        ui.label(egui::RichText::new("Token for the userscript").small());
        ui.horizontal(|ui| {
            let mut shown = token.to_string();
            ui.add(
                egui::TextEdit::singleline(&mut shown)
                    .desired_width(ui.available_width() - 56.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(token.to_string());
            }
        });
    }
}
