// ============================================================================
// Module:       strobe_app::ui::tags
// Description:  The tag row: a scene's tags as chips, or the field that edits
//               them, shared by the Inspector, the Import pane and the
//               browser's selected scene.
//
// Dependencies: egui; strobe-scene (tags::parse_list); super::{model, theme,
//               widgets}
// ============================================================================

//! One tag row for every place a scene is tagged (D205, D264).
//!
//! The Inspector tags the playing scene, the Import pane the scene it just
//! imported, and the browser the scene selected in it, playing or not. All
//! three draw this row and ask for the same [`Action::SetSceneTags`], naming
//! the scene by its library name, so a tag written from any of them is the
//! same write to that scene's `scene.json`.
//!
//! A draft belongs to the scene it was opened for ([`TagDraft::scene`]).
//! When that place shows another scene (the show moved on, another row was
//! selected), the draft is not shown there, so Save can never write one
//! scene's half-typed tags into another.

use super::model::Action;
use super::theme;
use super::widgets;
use crate::palette::Palette;
use egui::Ui;

/// The width the field leaves for its two buttons.
const BUTTONS: f32 = 120.0;

/// The narrowest the field gets.
const MIN_FIELD: f32 = 80.0;

/// A tag field being edited, and the scene it is for. Presentation state:
/// never saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagDraft {
    /// The scene's library name.
    pub scene: String,
    /// The field's text: tags separated by commas.
    pub text: String,
}

/// Where a tag row is, and what it draws besides the tags.
pub struct Place<'a> {
    /// The scene's library name.
    pub scene: &'a str,
    /// Whether to name the scene at the row's start, where nothing above
    /// says which scene it is (the browser, the Import pane).
    pub named: bool,
}

/// Draws the scene's `tags` as chips with an Edit button, or, while `draft`
/// is open for this scene, the field that edits them. `extra` draws more
/// buttons after Edit (the Inspector's "Use frame as picture").
pub fn row(
    ui: &mut Ui,
    theme: &Palette,
    place: Place<'_>,
    tags: &[String],
    draft: &mut Option<TagDraft>,
    extra: impl FnOnce(&mut Ui),
    actions: &mut Vec<Action>,
) {
    let scene = place.scene;
    let Some(editing) = draft.as_mut().filter(|draft| draft.scene == scene) else {
        ui.horizontal_wrapped(|ui| {
            // On the same line as the chips: a label of its own above them
            // would sit on their row's top in a short pane.
            if place.named {
                ui.label(
                    egui::RichText::new(scene)
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            }
            for tag in tags {
                widgets::chip(ui, tag, theme.raised, theme.text_muted);
            }
            if tags.is_empty() {
                ui.label(
                    egui::RichText::new("No tags")
                        .small()
                        .color(theme::rgb(theme.text_faint)),
                );
            }
            if ui
                .small_button(if tags.is_empty() { "Add tags" } else { "Edit" })
                .on_hover_text(
                    "Tags find the scene in the library and pick it when generating a playlist",
                )
                .clicked()
            {
                *draft = Some(TagDraft {
                    scene: scene.to_owned(),
                    text: tags.join(", "),
                });
            }
            extra(ui);
        });
        return;
    };
    let mut done = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut editing.text)
                .hint_text("dark, tunnel, audio reactive")
                .desired_width((ui.available_width() - BUTTONS).max(MIN_FIELD)),
        );
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.small_button("Save").clicked() || entered {
            actions.push(Action::SetSceneTags {
                scene: editing.scene.clone(),
                tags: strobe_scene::tags::parse_list(&editing.text),
            });
            done = true;
        }
        if ui.small_button("Cancel").clicked() {
            done = true;
        }
    });
    if done {
        *draft = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use egui::{Pos2, Vec2};

    /// One pass of the row for `scene` with tags `dark`; returns where each
    /// text was drawn.
    fn draw(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        scene: &str,
        draft: &mut Option<TagDraft>,
        actions: &mut Vec<Action>,
    ) -> Vec<(String, egui::Rect)> {
        let theme = crate::palette::fallback().clone();
        let tags = vec!["dark".to_string()];
        let pass = crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(480.0, 120.0), events),
            |ui| {
                let place = Place { scene, named: true };
                row(ui, &theme, place, &tags, draft, |_| {}, actions);
            },
        );
        pass.shapes
            .iter()
            .filter_map(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                Some((
                    text.galley.text().to_string(),
                    clipped.shape.visual_bounding_rect(),
                ))
            })
            .collect()
    }

    fn click(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_draft_shows_only_for_the_scene_it_was_opened_for() {
        let ctx = egui::Context::default();
        let mut draft = Some(TagDraft {
            scene: "01-aurora".to_string(),
            text: "dark, calm".to_string(),
        });
        let mut actions = Vec::new();
        let other = draw(&ctx, Vec::new(), "02-tunnel", &mut draft, &mut actions);
        assert!(
            other.iter().any(|(text, _)| text == "dark")
                && other.iter().all(|(text, _)| text != "Save"),
            "another scene shows its chips, not the draft: {other:?}"
        );
        assert!(draft.is_some(), "the draft is kept for its own scene");
        let own = draw(&ctx, Vec::new(), "01-aurora", &mut draft, &mut actions);
        assert!(
            own.iter().any(|(text, _)| text == "Save"),
            "its own scene shows the field: {own:?}"
        );
        assert!(actions.is_empty(), "drawing asks for nothing: {actions:?}");
    }

    #[test]
    fn save_tags_the_scene_the_draft_is_for() -> Result<()> {
        let ctx = egui::Context::default();
        let mut draft = Some(TagDraft {
            scene: "02-tunnel".to_string(),
            text: "Dark, audio reactive".to_string(),
        });
        let mut actions = Vec::new();
        let drawn = draw(&ctx, Vec::new(), "02-tunnel", &mut draft, &mut actions);
        let Some(save) = drawn
            .iter()
            .find(|(text, _)| text == "Save")
            .map(|(_, rect)| rect.center())
        else {
            anyhow::bail!("no Save button: {drawn:?}");
        };
        let _ = draw(
            &ctx,
            vec![egui::Event::PointerMoved(save), click(save, true)],
            "02-tunnel",
            &mut draft,
            &mut actions,
        );
        let _ = draw(
            &ctx,
            vec![click(save, false)],
            "02-tunnel",
            &mut draft,
            &mut actions,
        );
        assert_eq!(
            actions,
            vec![Action::SetSceneTags {
                scene: "02-tunnel".to_string(),
                tags: vec!["dark".to_string(), "audio-reactive".to_string()],
            }],
            "one write, to that scene, with the tags normalised"
        );
        assert!(draft.is_none(), "the field closes");
        Ok(())
    }
}
