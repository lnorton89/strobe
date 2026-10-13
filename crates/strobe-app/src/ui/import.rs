// ============================================================================
// Module:       strobe_app::ui::import
// Description:  The Shadertoy import pane: a path/id field, the result line,
//               and the browser-listener toggle with its token. The "replace
//               this scene?" question is the shell's modal.
//
// Dependencies: egui; super::{model, theme, widgets}; crate::{import, workspace}
// ============================================================================

//! The import pane (D110 to D117), in the left dock beside the browser.
//!
//! It only edits the model's text field and pushes [`Action`]s; the app owns
//! the worker, the listener and the library. A drop anywhere on the window
//! is handled by the app, so this pane just says "drop here" while a file is
//! being dragged over. The one place it asks a question, whether to replace a
//! scene that is already in the library (D113), it does through the shell's
//! confirmation card ([`super::modal`]) rather than a line of buttons that
//! is easy to miss.

use super::icon as icons;
use super::model::{Action, ImportPanel, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::import::Status;
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::Ui;

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(ui, theme, Icon::Import, Pane::Import.title(), |_| {});
    if closed {
        model.ui.panes.close(Pane::Import);
    }
    let panel = &mut model.import;
    ui.label(
        egui::RichText::new("Import from Shadertoy")
            .strong()
            .color(theme::rgb(theme.text)),
    );
    if panel.drop_hover {
        ui.colored_label(
            theme::rgb(theme.accent),
            "Drop an exported .json to import it",
        );
    }
    ui.add_space(SPACE_XS);
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(panel.text)
                .hint_text("file path, shader id or URL")
                .desired_width(ui.available_width() - 84.0),
        );
        let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ready = !panel.text.trim().is_empty() && !panel.busy;
        let clicked = ui
            .add_enabled_ui(ready, |ui| widgets::primary_button(ui, theme, "Import"))
            .inner
            .on_hover_text(
                "An exported Shadertoy .json file (you can also drop one on this window), \
                 or a shader id/URL to fetch with your API key.",
            )
            .clicked();
        if (clicked || submitted) && ready {
            actions.push(Action::Import(panel.text.trim().to_string()));
        }
    });
    status_line(ui, theme, panel.status, panel.busy);
    ui.add_space(SPACE_SM);
    widgets::section(ui, theme, "Browser userscript");
    listener(ui, theme, panel, actions);
}

fn status_line(ui: &mut Ui, theme: &Palette, status: &Status, busy: bool) {
    let (text, tone) = match status {
        Status::Idle => return,
        Status::Working(text) => (text.as_str(), None),
        Status::Done(text) => (text.as_str(), Some(Tone::Info)),
        Status::Warned(text) => (text.as_str(), Some(Tone::Warning)),
        Status::Failed(text) => (text.as_str(), Some(Tone::Error)),
    };
    ui.add_space(SPACE_XS);
    match tone {
        Some(tone @ (Tone::Warning | Tone::Error)) => {
            widgets::banner(ui, theme, tone, text, |_| {});
        }
        Some(Tone::Info) | None => {
            ui.horizontal_wrapped(|ui| {
                if busy {
                    ui.spinner();
                } else {
                    icons::show(ui, Icon::Check, icons::INLINE, theme::rgb(theme.success));
                }
                ui.label(
                    egui::RichText::new(text)
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
    }
}

fn listener(ui: &mut Ui, theme: &Palette, panel: &ImportPanel<'_>, actions: &mut Vec<Action>) {
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
            .color(theme::rgb(if panel.listener.failed {
                theme.danger
            } else {
                theme.text_muted
            })),
    );
    if let Some(token) = panel.listener.token {
        ui.label(egui::RichText::new("Token for the userscript").small());
        ui.horizontal(|ui| {
            let mut shown = token.to_string();
            ui.add(
                egui::TextEdit::singleline(&mut shown)
                    .desired_width(ui.available_width() - 64.0)
                    .font(egui::TextStyle::Monospace),
            );
            if widgets::button(ui, theme, Some(Icon::Copy), "Copy").clicked() {
                ui.ctx().copy_text(token.to_string());
            }
        });
    }
}

/// The question the replace confirmation asks, for the shell's modal.
pub fn replace_question(model: &Model<'_>) -> Option<super::modal::Confirmation> {
    let title = model.import.replace_title?;
    Some(super::modal::Confirmation {
        title: format!("Replace \"{title}\"?"),
        body: "This shader is already in the library. Replacing it swaps its \
               folder for a fresh import; if the import fails, the old scene \
               stays. Keeping it leaves the library as it was."
            .to_string(),
        confirm: "Replace scene".to_string(),
        cancel: "Keep existing".to_string(),
        destructive: true,
    })
}
