// ============================================================================
// Module:       strobe_app::ui::inspector
// Description:  The control inspector pane: the playing scene's controls,
//               grouped by UI_GROUP, with per-group default and random.
//
// Dependencies: egui; strobe-scene (ControlBank); super::{controls, model,
//               theme, widgets, state}, crate::script
// ============================================================================

//! The control inspector (UI_DESIGN sections 9 to 11, UI_IMPLEMENTATION_PLAN
//! section 10).
//!
//! The inspector is *generated* from the scene's `ControlDef`s: nothing here
//! knows what a control is called or what range it has, and there is no
//! second taxonomy. Groups come from `UI_GROUP`, in the order a scene
//! declares them, ungrouped controls first.
//!
//! Each group has a header with a disclosure chevron (collapse state is the
//! operator's and is saved), and two small actions that act on the group
//! only: *default* (every control in the group eases back to its default,
//! and a bang is left alone) and *random* (every control in the group moves
//! to a random point in its range). Descriptions stay in tooltips until
//! asked for; there is no four-button toolbar beside every control.
//!
//! States: no scene ("Pick a scene"), a scene with no controls, a script
//! that is running, faulted or disabled (visible here without opening the
//! console).

use super::controls;
use super::icon as icons;
use super::model::{Action, GeneratePanel, Model};
use super::presets;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::generate::{Kept, Outcome, Summary};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::script::ScriptState;
use crate::workspace::Pane;
use egui::{Sense, Ui, Vec2};
use strobe_scene::control::{ControlBank, ControlKind};

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let has_scene = model.controls.is_some();
    let closed = widgets::pane_header(ui, theme, Icon::Controls, Pane::Inspector.title(), |ui| {
        if has_scene
            && widgets::button(ui, theme, Some(Icon::Refresh), "Reload")
                .on_hover_text("Reload the scene from disk. Control values are kept.")
                .clicked()
        {
            actions.push(Action::Reload);
        }
    });
    if closed {
        model.ui.panes.close(Pane::Inspector);
    }

    if let Some(title) = (!model.scene_title.is_empty()).then_some(model.scene_title) {
        ui.label(
            egui::RichText::new(title)
                .strong()
                .color(theme::rgb(theme.text)),
        );
    }
    if let Some(state) = model.script {
        script_status(ui, theme, state);
    }
    ui.add_space(SPACE_XS);
    if has_scene {
        presets::show(ui, model, theme, actions);
        generate_strip(ui, theme, model.generate, actions);
    }

    let ui_state = &mut *model.ui;
    let meta = model.meta.as_deref_mut();
    match model.controls.as_deref_mut() {
        Some(bank) if !bank.defs().is_empty() => {
            groups(ui, theme, bank, meta, ui_state);
        }
        Some(_) => {
            widgets::empty_state(
                ui,
                theme,
                Icon::Controls,
                "This scene has no controls",
                "It plays as authored. Generate controls makes a copy with some.",
            );
            if let Some(meta) = meta {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| meta_section(ui, theme, meta, ui_state));
            }
        }
        None => widgets::empty_state(
            ui,
            theme,
            Icon::Library,
            "No scene playing",
            "Open the scene library and load one to see its controls.",
        ),
    }
}

/// "Generate controls": the button, the worker's progress, or its result
/// for review (D202). The worker runs in its own process, so the show keeps
/// playing; the result is a new scene, and the source is never changed.
fn generate_strip(
    ui: &mut Ui,
    theme: &Palette,
    panel: GeneratePanel<'_>,
    actions: &mut Vec<Action>,
) {
    if let Some((scene, seconds)) = panel.running {
        widgets::banner(
            ui,
            theme,
            Tone::Info,
            &format!("Measuring {scene} for controls ({seconds:.0} s). The show keeps playing."),
            |ui| {
                if widgets::button(ui, theme, None, "Cancel").clicked() {
                    actions.push(Action::CancelGenerate);
                }
            },
        );
        ui.add_space(SPACE_XS);
        return;
    }
    match panel.outcome {
        Some(Outcome::Done(summary)) => review(ui, theme, summary, actions),
        Some(Outcome::Failed { message }) => {
            widgets::banner(
                ui,
                theme,
                Tone::Error,
                &format!("No controls generated: {message}"),
                |ui| {
                    if widgets::button(ui, theme, None, "Dismiss").clicked() {
                        actions.push(Action::DismissGenerated);
                    }
                },
            );
            ui.add_space(SPACE_XS);
        }
        None => {
            let (label, tip) = if panel.authored {
                (
                    "Add generated controls",
                    "Measure the scene's literal numbers in the background and write a copy,                      <name>-auto, with the strongest ones added beside the controls it has.                      This scene is not changed.",
                )
            } else {
                (
                    "Generate controls",
                    "Measure the scene's literal numbers in the background and write a copy,                      <name>-auto, with the strongest ones as controls. This scene is not changed.",
                )
            };
            if widgets::button(ui, theme, Some(Icon::Controls), label)
                .on_hover_text(tip)
                .clicked()
            {
                actions.push(Action::GenerateControls);
            }
            ui.add_space(SPACE_XS);
        }
    }
}

/// The worker's result: what it made, how faithful the copy is, and the two
/// ways on.
fn review(ui: &mut Ui, theme: &Palette, summary: &Summary, actions: &mut Vec<Action>) {
    theme::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let Some(written) = summary.written.as_ref() else {
            ui.label(
                egui::RichText::new(format!(
                    "No useful controls in {}: {} literal(s) measured, none moved the picture enough.",
                    summary.scene, summary.candidates
                ))
                .color(theme::rgb(theme.text)),
            );
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissGenerated);
            }
            return;
        };
        let name = written
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(".synScene").to_owned())
            .unwrap_or_default();
        ui.label(
            egui::RichText::new(format!(
                "{} control(s) for {}, written as {name}",
                summary.kept.len(),
                summary.scene
            ))
            .strong()
            .color(theme::rgb(theme.text)),
        );
        let faithful = summary
            .fidelity
            .map_or_else(|| "not measured".to_owned(), |f| format!("{:.1}% off", f * 100.0));
        ui.label(
            egui::RichText::new(format!(
                "At their defaults the copy matches the original ({faithful}). This scene is unchanged."
            ))
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(SPACE_XS);
        let strongest = summary
            .kept
            .iter()
            .filter_map(|k| k.score)
            .fold(f32::EPSILON, f32::max);
        for kept in &summary.kept {
            ui.horizontal(|ui| {
                // The bar first, in a fixed column, so a long colour range
                // never runs under it.
                let fraction = kept.score.unwrap_or(0.0) / strongest;
                let _ = widgets::meter(ui, theme, fraction, theme.accent, 36.0)
                    .on_hover_text("How much the picture moves with it, against the strongest");
                ui.label(
                    egui::RichText::new(&kept.name)
                        .monospace()
                        .color(theme::rgb(theme.text)),
                );
                ui.label(
                    egui::RichText::new(format!("{} · {}", kept.group, range(kept)))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                )
                .on_hover_text(&kept.context);
            });
        }
        for line in &summary.suggestions {
            ui.label(
                egui::RichText::new(format!("Audio idea: {line}"))
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
        ui.add_space(SPACE_XS);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, theme, &format!("Open {name}")).clicked() {
                actions.push(Action::OpenGenerated);
            }
            if widgets::button(ui, theme, None, "Discard copy")
                .on_hover_text("Delete the copy. The original scene is not touched.")
                .clicked()
            {
                actions.push(Action::DiscardGenerated);
            }
        });
    });
    ui.add_space(SPACE_XS);
}

/// `0.5 (0–2)` for a slider, `(0.8, 0.3, 0.1)` for a colour.
fn range(kept: &Kept) -> String {
    let join = |values: &[f32]| {
        values
            .iter()
            .map(|v| format!("{v:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (
        kept.default.as_slice(),
        kept.min.as_slice(),
        kept.max.as_slice(),
    ) {
        ([value], [min], [max]) => format!("{value:.2} ({min:.2}–{max:.2})"),
        (values, _, _) => format!("({})", join(values)),
    }
}

/// One line (or a short paragraph) on how the scene's script is doing: a
/// dead script must be visible without opening the console.
fn script_status(ui: &mut Ui, theme: &Palette, state: &ScriptState) {
    match state {
        ScriptState::Running => {
            ui.horizontal(|ui| {
                icons::show(
                    ui,
                    Icon::Check,
                    icons::DISCLOSURE,
                    theme::rgb(theme.success),
                );
                ui.label(
                    egui::RichText::new("script.js running")
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
        ScriptState::Faulted {
            consecutive,
            message,
        } => widgets::banner(
            ui,
            theme,
            Tone::Warning,
            &format!("script.js fault ({consecutive} in a row): {message}"),
            |_| {},
        ),
        ScriptState::Disabled { message } => widgets::banner(
            ui,
            theme,
            Tone::Error,
            &format!("script.js disabled: {message}. Reload the scene to run it again."),
            |_| {},
        ),
    }
}

fn groups(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    meta: Option<&mut ControlBank>,
    state: &mut super::state::UiState,
) {
    let groups = controls::group_order(bank.defs());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (group, members) in groups {
                match group {
                    None => controls::rows(ui, theme, bank, &members),
                    Some(name) => {
                        let collapsed = state.is_collapsed(&name);
                        match group_header(ui, theme, &name, collapsed, members.len()) {
                            HeaderAction::None => {}
                            HeaderAction::Toggle => state.set_collapsed(&name, !collapsed),
                            HeaderAction::Default => {
                                for &index in &members {
                                    bank.set_default(index);
                                }
                            }
                            HeaderAction::Random => {
                                for &index in &members {
                                    bank.randomize(index, || state.random_unit());
                                }
                            }
                        }
                        if !collapsed {
                            ui.indent(("group", &name), |ui| {
                                controls::rows(ui, theme, bank, &members);
                            });
                        }
                    }
                }
                ui.add_space(SPACE_SM);
            }
            // Bangs are left alone by default and random; say so once, near
            // the end, only if the scene has any.
            let has_bang = bank
                .defs()
                .iter()
                .any(|def| matches!(def.kind, ControlKind::Bang(_)));
            if has_bang {
                ui.label(
                    egui::RichText::new("Default and random leave bangs alone.")
                        .small()
                        .color(theme::rgb(theme.text_faint)),
                );
            }
            if let Some(meta) = meta {
                meta_section(ui, theme, meta, state);
            }
        });
}

/// The output-wide meta controls (master, colour, mirror, audio gains),
/// drawn under the scene's own. They belong to the output, not the scene:
/// they are applied after the last pass (D180), and a script reaches them
/// with a `meta/` prefix. Collapsed by the operator, saved with the layout.
fn meta_section(
    ui: &mut Ui,
    theme: &Palette,
    meta: &mut ControlBank,
    state: &mut super::state::UiState,
) {
    const NAME: &str = "Output (meta)";
    let collapsed = state.is_collapsed(NAME);
    ui.separator();
    ui.horizontal(|ui| {
        let label = if collapsed {
            "Output (meta): show"
        } else {
            "Output (meta): hide"
        };
        if widgets::button(ui, theme, Some(Icon::ChevronRight), label)
            .on_hover_text("Master, colour, mirror and audio gains for the whole output")
            .clicked()
        {
            state.set_collapsed(NAME, !collapsed);
        }
        if widgets::button(ui, theme, Some(Icon::Reset), "Reset")
            .on_hover_text("Return every output control to neutral")
            .clicked()
        {
            for index in 0..meta.defs().len() {
                meta.set_default(index);
            }
        }
    });
    if collapsed {
        return;
    }
    for (group, members) in controls::group_order(meta.defs()) {
        if let Some(name) = group {
            ui.label(
                egui::RichText::new(name.to_uppercase())
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
        controls::rows(ui, theme, meta, &members);
        ui.add_space(SPACE_XS);
    }
}

/// What a group header's click did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderAction {
    None,
    Toggle,
    Default,
    Random,
}

/// A group header: a chevron that turns, the name, the member count, and
/// the group's two actions.
fn group_header(
    ui: &mut Ui,
    theme: &Palette,
    name: &str,
    collapsed: bool,
    count: usize,
) -> HeaderAction {
    let mut action = HeaderAction::None;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let id = ui.id().with(("group-header", name));
    let fill = widgets::hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.panel,
        theme.raised,
    );
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
    // Collapsed points right; open points down: a quarter turn.
    let turn = super::motion::toggle(ui.ctx(), id, !collapsed, super::motion::QUICK);
    icons::paint_rotated(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            Vec2::splat(icons::DISCLOSURE),
        ),
        Icon::ChevronRight,
        theme::rgb(theme.text_muted),
        turn * 0.25,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name.to_uppercase(),
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text),
    );
    ui.painter().text(
        egui::pos2(
            rect.left() + 24.0 + 7.0 * name.chars().count() as f32 + 14.0,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        count.to_string(),
        egui::FontId::new(10.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text_faint),
    );
    if response.clicked() {
        action = HeaderAction::Toggle;
    }
    // The two actions sit over the right end of the header and take their
    // clicks before the header does.
    let buttons = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 56.0, rect.top() + 2.0),
        egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if widgets::icon_button(
                ui,
                theme,
                Icon::Refresh,
                "Random: move every control in this group to a random value",
            )
            .clicked()
            {
                action = HeaderAction::Random;
            }
            if widgets::icon_button(
                ui,
                theme,
                Icon::Reset,
                "Default: return every control in this group to its default",
            )
            .clicked()
            {
                action = HeaderAction::Default;
            }
        },
    );
    action
}
