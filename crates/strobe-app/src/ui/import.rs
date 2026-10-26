// ============================================================================
// Module:       strobe_app::ui::import
// Description:  The Shadertoy import pane: the file, id or URL with Browse
//               and Import, the result, generating controls for what was
//               imported, its tags, and the browser listener. The "replace
//               this scene?" question is the shell's modal.
//
// Dependencies: egui; super::{form, model, theme, widgets, tags}; crate::{
//               import, picker, workspace}
// ============================================================================

//! The import pane (D110 to D117), in `super::form`'s grammar (UI review
//! step 3, D470).
//!
//! It only edits the model's text field and pushes [`Action`]s; the app owns
//! the worker, the listener and the library. A file can come three ways:
//! typed, chosen with Browse (the system's dialog, run off the frame loop),
//! or dropped anywhere on the window, which the app handles; this pane says
//! "drop it" while a file is being dragged over. The one place it asks a
//! question, whether to replace a scene that is already in the library
//! (D113), it does through the shell's confirmation card ([`super::modal`])
//! rather than a line of buttons that is easy to miss.
//!
//! The scene it last imported can be tagged here, with the same row as the
//! Inspector's, so a fresh import is findable in the library and by
//! Generate without loading it again (D264).

use super::form::{self, Row};
use super::model::{Action, ImportPanel, Model};
use super::tags;
use super::theme;
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::import::Status;
use crate::palette::Palette;
use crate::picker::Purpose;
use crate::workspace::Pane;
use egui::Ui;

/// The source row's title.
const SOURCE: &str = "Shader";

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(ui, theme, Icon::Import, Pane::Import.title(), |_| {});
    if closed {
        model.ui.panes.close(Pane::Import);
    }
    let picking = model.picking;
    egui::ScrollArea::vertical()
        .id_salt("import-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            form::page(
                ui,
                theme,
                "Import",
                Some("Shadertoy shaders become scenes in your library, ready to play and edit."),
                |ui| {
                    shadertoy(ui, model, theme, picking, actions);
                    listener(ui, theme, &model.import, actions);
                    ui.add_space(theme::SPACE_LG);
                },
            );
        });
}

/// The Shadertoy group: the source with Browse and Import, the result, and
/// what to do with the scene just imported.
fn shadertoy(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    picking: bool,
    actions: &mut Vec<Action>,
) {
    let mut generate = false;
    form::group(ui, theme, "Shadertoy", |group| {
        let panel = &mut model.import;
        let mut row = Row::new(SOURCE).description(
            "An exported .json file, or a shader's id or URL to fetch with your API key. A file \
             can also be dropped anywhere on this window.",
        );
        if panel.drop_hover {
            row = row.note("Drop the exported .json to import it.", theme.accent);
        }
        group.wide_row(row, |ui| {
            let ready = !panel.text.trim().is_empty() && !panel.busy;
            let clicked = ui
                .add_enabled_ui(ready, |ui| widgets::primary_button(ui, theme, "Import"))
                .inner
                .clicked();
            if widgets::browse_button(ui, theme, picking || panel.busy) {
                actions.push(Action::Browse(Purpose::Shadertoy));
            }
            let width = ui.available_size_before_wrap().x;
            let field = form::field(ui, theme, panel.text, "file, shader id or URL", width);
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (clicked || submitted) && ready {
                actions.push(Action::Import(panel.text.trim().to_string()));
            }
        });
        status(group, theme, panel.status, panel.busy);
        if panel.imported_playing {
            generate = generate_row(group, theme, panel.generating);
        }
        if let Some(entry) = panel
            .imported
            .and_then(|index| model.library.entries.get(index))
        {
            group.block(|ui| {
                tags::row(
                    ui,
                    theme,
                    tags::Place {
                        scene: &entry.name,
                        named: true,
                        read_only: entry.read_only().then_some(entry.dir.as_path()),
                    },
                    &entry.tags,
                    &mut model.ui.import_tag_draft,
                    |_| {},
                    actions,
                );
            });
        }
    });
    if generate {
        actions.push(Action::GenerateControls);
        // Opened now, so the review the worker ends in is in view.
        model.ui.panes.open(Pane::Inspector);
    }
}

/// "Generate controls" for the scene just imported (U-03, D246): the same
/// worker as the Inspector's button, whose review then shows there. Only
/// while that scene is the one playing, since the worker measures it.
/// Whether it was clicked.
fn generate_row(group: &mut form::Group<'_>, theme: &Palette, generating: bool) -> bool {
    let mut clicked = false;
    let row = Row::new("Generate controls").description(if generating {
        "Measuring it now; the result appears in the Inspector to review."
    } else {
        "Measure the imported scene's literal numbers and write a copy, <name>-auto, with the \
         strongest ones as controls, to review in the Inspector. The import is not changed."
    });
    group.row(row.enabled(!generating), |ui| {
        clicked = widgets::button(ui, theme, Some(Icon::Controls), "Generate").clicked();
    });
    clicked
}

/// The last import's outcome: a line while it works and when it is done, a
/// banner when it warns or fails.
fn status(group: &mut form::Group<'_>, theme: &Palette, status: &Status, busy: bool) {
    let (text, tone) = match status {
        Status::Idle => return,
        Status::Working(text) => (text.as_str(), None),
        Status::Done(text) => (text.as_str(), Some(Tone::Info)),
        Status::Warned(text) => (text.as_str(), Some(Tone::Warning)),
        Status::Failed(text) => (text.as_str(), Some(Tone::Error)),
    };
    group.block(|ui| match tone {
        Some(tone @ (Tone::Warning | Tone::Error)) => {
            widgets::banner(ui, theme, tone, text, |_| {});
        }
        Some(Tone::Info) | None => {
            ui.horizontal_wrapped(|ui| {
                if busy {
                    ui.spinner();
                } else {
                    widgets::status_chip(ui, Icon::Check, "Imported", theme.success);
                }
                ui.label(
                    egui::RichText::new(text)
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
    });
}

fn listener(ui: &mut Ui, theme: &Palette, panel: &ImportPanel<'_>, actions: &mut Vec<Action>) {
    form::group(ui, theme, "Browser userscript", |group| {
        let mut enabled = panel.listener.enabled;
        let row = Row::new("Accept shaders from the browser")
            .description(
                "Listens on this computer only. The \"Send to Strobe\" userscript on \
                 shadertoy.com posts a shader here with the token below.",
            )
            .note(
                panel.listener.state,
                if panel.listener.failed {
                    theme.danger
                } else {
                    theme.text_muted
                },
            );
        if group.toggle(row, &mut enabled).changed() {
            actions.push(Action::SetImportListener(enabled));
        }
        if let Some(token) = panel.listener.token {
            group.wide_row(
                Row::new("Token").description("Paste it into the userscript's settings."),
                |ui| {
                    if widgets::button(ui, theme, Some(Icon::Copy), "Copy").clicked() {
                        ui.ctx().copy_text(token.to_string());
                    }
                    widgets::keycap(ui, theme, token, theme.text);
                },
            );
        }
    });
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
