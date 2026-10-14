// ============================================================================
// Module:       strobe_app::ui::presets
// Description:  The preset row of the side panel's Presets tab (recall a
//               preset, save the controls as one, rename) and the fav row
//               under the preview: eight slots for scene + preset pairs.
//
// Dependencies: egui; super::{model, theme, widgets}, crate::icon
// ============================================================================

//! Presets and fav slots (D176, D283).
//!
//! ```text
//! PRESET  [ Slow Burn          v ]  [name         ] [Save]  [Delete]
//! FAV     [1] [2] [3] [ ] [ ] [ ] [ ] [ ]
//! ```
//!
//! The fav row is the top of the side panel's Presets tab and the preset row
//! sits under it (D283).
//!
//! A **preset** is the scene's control values under a name, kept in the
//! scene folder. Choosing one in the list recalls it at once (the values are
//! written as targets, so smoothed controls ease to them). *Save* captures
//! the controls as the name typed, replacing a preset of that name.
//!
//! A **fav slot** holds a scene and optionally one of its presets. A filled
//! slot recalls it with a click (loading the scene first if it is not the
//! one playing); an empty one stores the playing scene and its active preset.
//! Right-click a slot to store into it or to clear it. The same slots are
//! reachable from MIDI, OSC and the macro strip.
//!
//! The pane only asks: every action goes back to the app, which owns the
//! files and the scene.

use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::palette::Palette;
use egui::Ui;

/// Draws the preset row: the picker, delete, and save or rename.
pub fn preset_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let panel = &mut model.remote;
    let names = panel.presets;
    let active = panel.active_preset;
    ui.horizontal(|ui| {
        label(ui, theme, "PRESET");
        let shown = active.unwrap_or(if names.is_empty() {
            "none saved"
        } else {
            "choose"
        });
        let delete_room = if active.is_some() { 34.0 } else { 0.0 };
        egui::ComboBox::from_id_salt("preset-pick")
            .width((ui.available_width() - delete_room - SPACE_SM).max(80.0))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for name in names {
                    if ui
                        .selectable_label(active == Some(name.as_str()), name)
                        .clicked()
                    {
                        actions.push(Action::RecallPreset(name.clone()));
                    }
                }
            });
        if let Some(active) = active {
            // Deleting is two clicks: the first arms it and says so (P-07).
            let armed = panel.ui.delete_armed.as_deref() == Some(active);
            let tip = if armed {
                "Click again to delete this preset's file"
            } else {
                "Delete the active preset's file (asks for a second click)"
            };
            if widgets::icon_button(ui, theme, Icon::Close, tip).clicked() {
                if armed {
                    actions.push(Action::DeletePreset(active.to_string()));
                    panel.ui.delete_armed = None;
                } else {
                    panel.ui.delete_armed = Some(active.to_string());
                }
            }
        }
        if panel.ui.delete_armed.as_deref() != active {
            panel.ui.delete_armed = None;
        }
    });
    if panel.ui.delete_armed.is_some() {
        ui.label(
            egui::RichText::new("Click the cross again to delete the preset.")
                .small()
                .color(theme::rgb(theme.warning)),
        );
    }
    ui.add_space(SPACE_XS);
    ui.horizontal(|ui| {
        let can_rename = active.is_some();
        let save_room = if can_rename { 140.0 } else { 72.0 };
        let field = &mut panel.ui.preset_name;
        ui.add(
            egui::TextEdit::singleline(field)
                .hint_text("save as...")
                .desired_width((ui.available_width() - save_room - SPACE_SM).max(60.0)),
        );
        let name = field.trim().to_string();
        let usable = strobe_scene::preset::valid_name(&name);
        // Saving over an existing name says so before it happens (P-07).
        let existing = names.iter().find(|n| n.eq_ignore_ascii_case(&name));
        let (label, tip) = match existing {
            Some(existing) => (
                "Replace",
                format!("Replace the preset \"{existing}\" with the controls as they are now"),
            ),
            None => (
                "Save",
                "Save the controls as a new preset of this scene".to_string(),
            ),
        };
        let save = ui
            .add_enabled_ui(usable, |ui| {
                widgets::button(ui, theme, None, label).on_hover_text(tip)
            })
            .inner;
        if save.clicked() {
            actions.push(Action::SavePreset(name.clone()));
        }
        if let Some(active) = active {
            let can = usable && existing.is_none_or(|e| e.eq_ignore_ascii_case(active))
                && name != active;
            let rename = ui
                .add_enabled_ui(can, |ui| {
                    widgets::button(ui, theme, None, "Rename").on_hover_text(format!(
                        "Rename \"{active}\" to the name typed here; fav slots that recall it follow"
                    ))
                })
                .inner;
            if rename.clicked() {
                actions.push(Action::RenamePreset {
                    from: active.to_string(),
                    to: name,
                });
            }
        }
    });
    ui.add_space(SPACE_XS);
}

/// Draws the fav row: eight slots sized to share the width, so the row is
/// one line at every panel width.
pub fn fav_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let panel = &mut model.remote;
    ui.horizontal(|ui| {
        // Tight: eight slots and their label must fit the narrowest drawer
        // on one line, so the slots never land under the tab bar.
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        ui.add_sized(
            [FAV_LABEL, 18.0],
            egui::Label::new(
                egui::RichText::new("FAV")
                    .size(10.0)
                    .strong()
                    .color(theme::rgb(theme.text_muted)),
            ),
        );
        let slots = strobe_scene::preset::FAV_SLOTS;
        let slot_width = ((ui.available_width() - SPACE_XS * (slots as f32 - 1.0)) / slots as f32)
            .clamp(14.0, 40.0);
        for slot in 0..slots {
            let fav = panel.remote.favs.get(slot);
            let filled = fav.is_some();
            let here = fav.is_some_and(|f| Some(f.scene.as_str()) == panel.scene_name);
            let tip = match fav {
                Some(f) => format!(
                    "Slot {}: {}{}. Click to recall. Right-click to store here or clear.",
                    slot + 1,
                    f.scene,
                    f.preset
                        .as_deref()
                        .map_or_else(String::new, |p| format!(" with preset {p}"))
                ),
                None => format!(
                    "Slot {} is empty. Click to store the playing scene here.",
                    slot + 1
                ),
            };
            let colour = if filled {
                theme.accent
            } else {
                theme.text_faint
            };
            let text = egui::RichText::new((slot + 1).to_string())
                .strong()
                .color(theme::rgb(colour));
            let response = ui
                .add_sized(
                    egui::vec2(slot_width, 24.0),
                    egui::Button::new(text).selected(here),
                )
                .on_hover_text(tip);
            if response.clicked() {
                actions.push(if filled {
                    Action::RecallFav(slot)
                } else {
                    Action::StoreFav(slot)
                });
            }
            response.context_menu(|ui| {
                if ui.button("Store the playing scene here").clicked() {
                    actions.push(Action::StoreFav(slot));
                    ui.close();
                }
                if filled && ui.button("Clear this slot").clicked() {
                    actions.push(Action::ClearFav(slot));
                    ui.close();
                }
            });
        }
    });
}

/// The fav row's label column.
const FAV_LABEL: f32 = 28.0;

fn label(ui: &mut Ui, theme: &Palette, text: &str) {
    ui.add_sized(
        [40.0, 18.0],
        egui::Label::new(
            egui::RichText::new(text)
                .size(10.0)
                .strong()
                .color(theme::rgb(theme.text_muted)),
        ),
    );
}
