// ============================================================================
// Module:       strobe_app::ui::controls
// Description:  The right panel: one widget per scene control, grouped by
//               UI_GROUP, writing the control bank's targets.
//
// Dependencies: egui; strobe-scene (ControlBank, ControlDef, ControlKind)
// ============================================================================

//! The scene's controls (right panel).
//!
//! One widget per [`ControlKind`], chosen by an exhaustive match so a new
//! control type cannot silently render nothing:
//!
//! | Control | Widget | Writes |
//! |---|---|---|
//! | slider, knob | slider over the control's real range | `set` |
//! | toggle | checkbox | `set` to `MIN` or `MAX` |
//! | bang, bang counter | button | `press` |
//! | xy | drag pad | `set` of both components |
//! | color | colour picker | `set` of three components |
//! | dropdown | combo box of labels | `set` to the option index |
//!
//! Widgets show the control's *target* (what was asked for), not its
//! smoothed output, so a smooth slider does not fight the hand moving it.

use crate::theme;
use egui::{Sense, Vec2};
use strobe_scene::control::{Bang, ControlBank, ControlDef, ControlKind};

/// Size of the xy pad, in points.
const PAD_SIZE: Vec2 = Vec2::new(180.0, 120.0);

/// Draws every control, ungrouped ones first, then each group in order of
/// first appearance.
pub fn show(ui: &mut egui::Ui, bank: &mut ControlBank) {
    let groups = group_order(bank.defs());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (group, members) in groups {
                match group {
                    None => draw_members(ui, bank, &members),
                    Some(name) => {
                        egui::CollapsingHeader::new(egui::RichText::new(&name).strong())
                            .id_salt(&name)
                            .default_open(true)
                            .show(ui, |ui| draw_members(ui, bank, &members));
                    }
                }
            }
        });
}

/// Control indices bucketed by `UI_GROUP`: the ungrouped bucket first,
/// then groups in the order they first appear.
fn group_order(defs: &[ControlDef]) -> Vec<(Option<String>, Vec<usize>)> {
    let mut groups: Vec<(Option<String>, Vec<usize>)> = vec![(None, Vec::new())];
    for (index, def) in defs.iter().enumerate() {
        let slot = groups.iter().position(|(name, _)| *name == def.group);
        match slot {
            Some(slot) => {
                if let Some((_, members)) = groups.get_mut(slot) {
                    members.push(index);
                }
            }
            None => groups.push((def.group.clone(), vec![index])),
        }
    }
    groups.retain(|(_, members)| !members.is_empty());
    groups
}

fn draw_members(ui: &mut egui::Ui, bank: &mut ControlBank, members: &[usize]) {
    for &index in members {
        let Some(def) = bank.defs().get(index).cloned() else {
            continue;
        };
        let target = bank.target(index).copied().unwrap_or_default();
        let output = bank.output(index).copied().unwrap_or_default();
        ui.push_id(index, |ui| match def.kind {
            ControlKind::Slider(_) | ControlKind::Knob(_) => {
                slider(ui, bank, index, &def, target[0]);
            }
            ControlKind::Toggle { .. } => toggle(ui, bank, index, &def, target[0]),
            ControlKind::Bang(kind) => bang(ui, bank, index, &def, kind, output[0]),
            ControlKind::Xy(_) => xy(ui, bank, index, &def, target),
            ControlKind::Color { .. } => color(ui, bank, index, &def, target),
            ControlKind::Dropdown { .. } => dropdown(ui, bank, index, &def, target[0]),
        });
        ui.add_space(2.0);
    }
}

fn describe(response: egui::Response, def: &ControlDef) -> egui::Response {
    match &def.description {
        Some(text) => response.on_hover_text(text),
        None => response,
    }
}

fn slider(ui: &mut egui::Ui, bank: &mut ControlBank, index: usize, def: &ControlDef, v: f32) {
    let (min, max) = (def.min[0], def.max[0]);
    if (max - min).abs() < f32::EPSILON {
        ui.label(format!("{}: {v:.3}", def.name));
        return;
    }
    let mut value = v;
    let response = ui.add(egui::Slider::new(&mut value, min..=max).text(&def.name));
    if describe(response, def).changed() {
        bank.set(index, Some(0), &[value]);
    }
}

fn toggle(ui: &mut egui::Ui, bank: &mut ControlBank, index: usize, def: &ControlDef, v: f32) {
    let mut on = def.normalize(0, v) >= 0.5;
    if describe(ui.checkbox(&mut on, &def.name), def).changed() {
        let next = if on { def.max[0] } else { def.min[0] };
        bank.set(index, Some(0), &[next]);
    }
}

fn bang(
    ui: &mut egui::Ui,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    kind: Bang,
    output: f32,
) {
    ui.horizontal(|ui| {
        if describe(ui.button(&def.name), def).clicked() {
            bank.press(index);
        }
        match kind {
            Bang::Counter | Bang::CounterSmooth => {
                ui.label(
                    egui::RichText::new(format!("{output:.0}"))
                        .monospace()
                        .color(theme::muted()),
                );
            }
            Bang::Plain | Bang::Smooth => {}
        }
    });
}

fn xy(ui: &mut egui::Ui, bank: &mut ControlBank, index: usize, def: &ControlDef, v: [f32; 3]) {
    ui.label(&def.name);
    let (rect, response) = ui.allocate_exact_size(PAD_SIZE, Sense::click_and_drag());
    let response = describe(response, def);
    if let Some(pos) = response.interact_pointer_pos() {
        let tx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let ty = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        bank.set(
            index,
            None,
            &[def.denormalize(0, tx), def.denormalize(1, ty)],
        );
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    painter.rect_stroke(
        rect,
        3.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let at = egui::pos2(
        rect.left() + def.normalize(0, v[0]) * rect.width(),
        rect.bottom() - def.normalize(1, v[1]) * rect.height(),
    );
    let guide = egui::Stroke::new(1.0, theme::muted());
    painter.hline(rect.x_range(), at.y, guide);
    painter.vline(at.x, rect.y_range(), guide);
    painter.circle_filled(at, 5.0, theme::accent());
    ui.label(
        egui::RichText::new(format!("{:.2}, {:.2}", v[0], v[1]))
            .monospace()
            .color(theme::muted()),
    );
}

fn color(ui: &mut egui::Ui, bank: &mut ControlBank, index: usize, def: &ControlDef, v: [f32; 3]) {
    ui.horizontal(|ui| {
        let mut rgb = v;
        if describe(ui.color_edit_button_rgb(&mut rgb), def).changed() {
            bank.set(index, None, &rgb);
        }
        ui.label(&def.name);
    });
}

fn dropdown(ui: &mut egui::Ui, bank: &mut ControlBank, index: usize, def: &ControlDef, v: f32) {
    let selected = (v.round().max(0.0) as usize).min(def.labels.len().saturating_sub(1));
    let current = def.labels.get(selected).cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        let response = egui::ComboBox::from_id_salt(("dropdown", index))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (option, label) in def.labels.iter().enumerate() {
                    if ui.selectable_label(option == selected, label).clicked() {
                        bank.set(index, Some(0), &[option as f32]);
                    }
                }
            });
        describe(response.response, def);
        ui.label(&def.name);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use strobe_scene::scene::SceneDef;

    #[test]
    fn controls_group_by_ui_group_in_first_seen_order() -> Result<()> {
        let json = r#"{ "CONTROLS": [
            { "NAME": "a", "TYPE": "slider", "UI_GROUP": "Zeta" },
            { "NAME": "b", "TYPE": "toggle" },
            { "NAME": "c", "TYPE": "slider", "UI_GROUP": "Alpha" },
            { "NAME": "d", "TYPE": "slider", "UI_GROUP": "Zeta" }
        ] }"#;
        let def = SceneDef::parse(json, "t")?;
        let groups = group_order(&def.controls);
        let shape: Vec<(Option<&str>, &[usize])> = groups
            .iter()
            .map(|(name, members)| (name.as_deref(), members.as_slice()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (None, &[1_usize][..]),
                (Some("Zeta"), &[0, 3][..]),
                (Some("Alpha"), &[2][..])
            ],
            "ungrouped first, then groups as first seen"
        );
        Ok(())
    }
}
