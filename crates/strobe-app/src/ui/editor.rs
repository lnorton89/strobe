// ============================================================================
// Module:       strobe_app::ui::editor
// Description:  The Editor pane: the scene's files as tabs, a highlighted
//               code editor, and Save / Save and reload / Revert / Restore,
//               with the compiler's error one click from its line.
//
// Dependencies: egui; crate::editor (documents, highlighting);
//               super::{model, theme, widgets}
// ============================================================================

//! The Editor workspace, drawn (UI_DESIGN section 14, D204).
//!
//! Typing changes nothing on screen; that is the point of the status line
//! ("Unsaved", "Saved, not reloaded", "Error", "Ready"). Save and reload is
//! the one action that reaches the show, and it goes through the same reload
//! the Controls pane's button does, so a scene that fails to compile leaves
//! the last good one playing.
//!
//! Keys while the code has focus: Ctrl+S saves, Ctrl+Enter saves and
//! reloads. Every other shortcut is off while typing (the focus rule in
//! [`crate::commands`]).
//!
//! An error's button goes to its line and, when the compiler gave one, its
//! column. A Save and reload refused because `scene.json` does not parse
//! shows the parser's message here, with its line and column, and nothing
//! reaches the show (D263).
//!
//! A scene of Synesthesia's opens like any other, but its status line says
//! it is read-only and a Copy to my library button leads the row (D271).
//! Save is still there and is refused by the app with the same offer, so
//! edits made before noticing are never lost: the copy carries them.

use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::editor::{self, Location, Token, error_location, location_offset};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Key, Modifiers, Ui};

/// What the Editor keeps between frames that is not a document.
#[derive(Debug, Default)]
pub struct EditorUi {
    /// A line, and a column when known, to put the cursor on, on the next
    /// frame.
    pub goto: Option<(usize, Option<usize>)>,
    /// The last highlight, keyed by the text's hash, so typing re-colours
    /// once per change rather than once per frame.
    cache: Option<(u64, LayoutJob)>,
}

/// Draws the pane.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(ui, theme, Icon::Editor, Pane::Editor.title(), |_| {});
    if closed {
        model.ui.panes.close(Pane::Editor);
    }
    let playing_dir = model.editor.playing_dir;
    let read_only = model.editor.read_only;
    let failure = model.failure.map(|f| (f.scene, f.summary));
    let Some(doc) = model.editor.doc.as_deref_mut() else {
        widgets::empty_state(
            ui,
            theme,
            Icon::Editor,
            "Nothing open",
            "Load a scene, then open it here to edit its shader, scene.json and script.",
        );
        if playing_dir.is_some()
            && widgets::primary_button(ui, theme, "Open the playing scene").clicked()
        {
            actions.push(Action::OpenEditor(None));
        }
        return;
    };

    // Keys first, so the code field never sees them.
    let (save, reload) = ui.ctx().input_mut(|input| {
        (
            input.consume_key(Modifiers::COMMAND, Key::S),
            input.consume_key(Modifiers::COMMAND, Key::Enter),
        )
    });
    if reload {
        actions.push(Action::EditorSaveReload);
    } else if save {
        actions.push(Action::EditorSave);
    }

    let on_screen = playing_dir.is_some_and(|dir| dir == doc.dir);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&doc.name)
                .strong()
                .color(theme::rgb(theme.text)),
        );
        if !on_screen {
            ui.label(
                egui::RichText::new("not the scene on screen")
                    .small()
                    .color(theme::rgb(theme.warning)),
            );
        }
    });

    // The failure, when it is this scene's.
    let error = failure
        .filter(|(scene, _)| *scene == doc.name)
        .map(|(_, summary)| summary);
    let (word, tone) = if read_only {
        (
            "Read-only: one of Synesthesia's scenes. Copy it to your library to save edits.",
            Tone::Warning,
        )
    } else if doc.invalid.is_some() {
        (
            "Not saved or reloaded: scene.json does not parse",
            Tone::Error,
        )
    } else if doc.is_dirty() {
        ("Unsaved changes", Tone::Warning)
    } else if error.is_some() {
        (
            "Error: the scene on screen is the last one that compiled",
            Tone::Error,
        )
    } else {
        ("Ready: what is on disk is what plays", Tone::Info)
    };
    ui.label(
        egui::RichText::new(word)
            .small()
            .color(theme::rgb(tone.colour(theme))),
    );
    ui.add_space(SPACE_XS);

    ui.horizontal_wrapped(|ui| {
        if read_only
            && widgets::primary_button(ui, theme, super::tags::COPY_LABEL)
                .on_hover_text(super::tags::READ_ONLY_HINT)
                .clicked()
        {
            actions.push(Action::CopyToLibrary(doc.dir.clone()));
        }
        if widgets::primary_button(ui, theme, "Save and reload")
            .on_hover_text("Write the files and load the scene again (Ctrl+Enter)")
            .clicked()
        {
            actions.push(Action::EditorSaveReload);
        }
        if widgets::button(ui, theme, None, "Save")
            .on_hover_text("Write the files; the show keeps the scene it has (Ctrl+S)")
            .clicked()
        {
            actions.push(Action::EditorSave);
        }
        if widgets::button(ui, theme, None, "Revert")
            .on_hover_text("Throw away what has not been saved")
            .clicked()
        {
            actions.push(Action::EditorRevert);
        }
        if widgets::button(ui, theme, None, "Restore backup")
            .on_hover_text(
                "Put this file's text from before this session's first save back in the editor \
                 (not saved until you save)",
            )
            .clicked()
        {
            actions.push(Action::EditorRestoreBackup);
        }
    });
    ui.add_space(SPACE_XS);

    if let Some(invalid) = doc.invalid.clone() {
        location_banner(
            ui,
            theme,
            &invalid.message,
            invalid.at.as_ref(),
            doc,
            model.ui,
        );
    }
    if let Some(summary) = error {
        let location = error_location(summary);
        location_banner(ui, theme, summary, location.as_ref(), doc, model.ui);
    }

    // Tabs.
    ui.horizontal_wrapped(|ui| {
        for (index, tab) in doc.tabs.iter().enumerate() {
            let label = if tab.is_dirty() {
                format!("{} \u{2022}", tab.file)
            } else {
                tab.file.clone()
            };
            if ui.selectable_label(index == doc.active, label).clicked() {
                doc.active = index;
            }
        }
    });
    ui.add_space(SPACE_SM);

    let active = doc.active.min(doc.tabs.len().saturating_sub(1));
    let invalid = &mut doc.invalid;
    let Some(tab) = doc.tabs.get_mut(active) else {
        return;
    };
    let glsl = tab.is_glsl();
    let id = egui::Id::new(("editor-code", doc.dir.as_path(), tab.file.as_str()));
    let editor_ui = &mut model.ui.editor;
    let goto = editor_ui.goto.take();
    let cache = &mut editor_ui.cache;
    // Code is not wrapped: a wrapped line reads as two, and line numbers in
    // errors would stop matching what is seen. The area scrolls sideways.
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap_width: f32| {
        let mut job = if glsl {
            highlighted(ui, theme, buffer.as_str(), cache)
        } else {
            LayoutJob::simple(
                buffer.as_str().to_owned(),
                egui::TextStyle::Monospace.resolve(ui.style()),
                theme::rgb(theme.text),
                f32::INFINITY,
            )
        };
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let output = egui::TextEdit::multiline(&mut tab.text)
                .id(id)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(24)
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);
            if output.response.changed() && tab.file == editor::SCENE_JSON {
                // The refusal was about the text before this edit.
                *invalid = None;
            }
            if let Some((line, column)) = goto {
                let offset = location_offset(&tab.text, line, column);
                let chars = tab.text.get(..offset).map_or(0, |s| s.chars().count());
                let mut state = output.state;
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(chars))));
                state.store(ui.ctx(), output.response.id);
                output.response.request_focus();
            }
        });
}

/// An error banner with a button to its place, when the place is a file
/// the Editor has open.
fn location_banner(
    ui: &mut Ui,
    theme: &Palette,
    message: &str,
    at: Option<&Location>,
    doc: &mut editor::Doc,
    model_ui: &mut super::state::UiState,
) {
    widgets::banner(ui, theme, Tone::Error, message, |ui| {
        if let Some(at) = at
            && let Some(index) = doc.tab_index(&at.file)
            && widgets::button(ui, theme, None, &format!("Go to {}", at.describe())).clicked()
        {
            doc.active = index;
            model_ui.editor.goto = Some((at.line, at.column));
        }
    });
    ui.add_space(SPACE_XS);
}

/// The colour of each kind of token.
fn colour(theme: &Palette, token: Token) -> egui::Color32 {
    theme::rgb(match token {
        Token::Plain => theme.text,
        Token::Comment => theme.text_faint,
        Token::Keyword => theme.accent,
        Token::Number => theme.warning,
        Token::Directive => theme.text_muted,
        Token::Builtin => theme.info,
    })
}

/// `text` coloured as GLSL, reusing the last result when the text is the
/// same.
fn highlighted(
    ui: &Ui,
    theme: &Palette,
    text: &str,
    cache: &mut Option<(u64, LayoutJob)>,
) -> LayoutJob {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let key = hasher.finish();
    if let Some((cached, job)) = cache.as_ref()
        && *cached == key
    {
        return job.clone();
    }
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob::default();
    for (start, end, token) in editor::highlight(text) {
        job.append(
            text.get(start..end).unwrap_or_default(),
            0.0,
            TextFormat::simple(font.clone(), colour(theme, token)),
        );
    }
    *cache = Some((key, job.clone()));
    job
}
