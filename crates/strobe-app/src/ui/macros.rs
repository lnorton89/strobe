// ============================================================================
// Module:       strobe_app::ui::macros
// Description:  The Performance Macros strip under the preview: the
//               operator's pinned macros, each a card that says what it acts
//               on, whether it can right now, and which MIDI or OSC source
//               drives it.
//
// Dependencies: egui; strobe-io (Macro, Target, State); strobe-scene
//               (ControlBank); super::{controls, map, model, theme, widgets},
//               crate::{remote, workspace}
// ============================================================================

//! Performance Macros, stage 2 (UI_DESIGN section 8, D175).
//!
//! A macro is a persistent, performance-facing binding: a name, a target
//! that is looked up in whatever scene is playing, a pin and a place in the
//! order. The strip draws the **pinned** macros in the operator's order.
//!
//! Each card shows:
//!
//! - a **source badge** for what it acts on (`SCENE`, `META`, `PLAYLIST`,
//!   `SYSTEM`) and its name;
//! - its **state** whenever it is not simply working: *unavailable in
//!   current scene* (muted, with the widget replaced by that sentence),
//!   *broken*, *disabled*, *learning*, *conflict*, each as a word and an
//!   icon;
//! - a **MIDI** or **OSC** tag with the controller that drives it, when one
//!   does;
//! - a widget fitted to the target: a rail for a slider or knob, a switch,
//!   a trigger button, a dropdown, a colour swatch, or a button for a
//!   command, a cue, a fav slot or a preset.
//!
//! The widget writes the same [`ControlBank`] entry the inspector and a
//! fader do, so there is one control model. Right-click a card for Learn,
//! Unpin and the order. A scene change never deletes or reorders a macro;
//! one whose control the new scene lacks goes muted until a scene that has
//! it plays.
//!
//! **With no macros at all** the strip falls back to its stage 1 behaviour,
//! the playing scene's first controls marked `SCENE`, so a new install is
//! not an empty bar. The moment one macro exists, only macros are shown.

use super::controls;
use super::map;
use super::model::Action;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::palette::Palette;
use crate::remote::Remote;
use crate::workspace::{Pane, Panes};
use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use strobe_io::target::{Flat, State, flatten, locate_control};
use strobe_io::{Macro, Resolution, Target};
use strobe_scene::control::{ControlBank, ControlKind};

/// The most scene controls the stage 1 fallback shows.
pub const MAX_MACROS: usize = 6;

/// The width of one macro cell, in points.
const CELL_WIDTH: f32 = 168.0;

/// The height of the strip.
pub const STRIP_HEIGHT: f32 = 60.0;

/// How many cells fit in `width`, between 1 and [`MAX_MACROS`].
pub fn cells_that_fit(width: f32, gap: f32) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    (((width + gap) / (CELL_WIDTH + gap)).floor() as usize).clamp(1, MAX_MACROS)
}

/// The first `count` controls of the bank, in declaration order.
pub fn pick(bank: &ControlBank, count: usize) -> Vec<usize> {
    (0..bank.defs().len()).take(count).collect()
}

/// What the strip needs, bundled so the call site stays readable.
pub struct Strip<'a> {
    /// The playing scene's controls.
    pub scene: Option<&'a mut ControlBank>,
    /// The meta controls.
    pub meta: Option<&'a mut ControlBank>,
    /// Show control: the macro set and the mappings.
    pub remote: &'a mut Remote,
    /// The playing scene's preset names.
    pub presets: &'a [String],
    /// The pane model, for the XY button.
    pub panes: &'a mut Panes,
}

/// What one card is about to draw, gathered before anything is mutated.
struct Card {
    found: Macro,
    resolution: Resolution,
    /// `MIDI` or `OSC`, when a controller drives it.
    tag: Option<String>,
    /// Which controllers, for the tooltip.
    tip: String,
}

/// Draws the strip.
pub fn strip(ui: &mut Ui, theme: &Palette, strip: Strip<'_>, actions: &mut Vec<Action>) {
    let Strip {
        mut scene,
        mut meta,
        remote,
        presets,
        panes,
    } = strip;
    if remote.engine.macros.is_empty() {
        legacy(ui, theme, scene, panes);
        return;
    }
    let evaluated = remote.evaluate(scene.as_deref(), meta.as_deref(), presets);
    let cards: Vec<Card> = remote
        .engine
        .macros
        .all()
        .iter()
        .zip(evaluated.macros)
        .filter(|(found, _)| found.pinned)
        .map(|(found, resolution)| Card {
            tag: remote
                .mappings_for_macro(found.id)
                .next()
                .map(|m| m.source.badge().to_string()),
            tip: remote
                .mappings_for_macro(found.id)
                .map(|m| m.source.label())
                .collect::<Vec<_>>()
                .join(", "),
            found: found.clone(),
            resolution,
        })
        .collect();
    if cards.is_empty() {
        placeholder(
            ui,
            theme,
            "Every macro is unpinned. Pin one in the MAP pane to put it here.",
        );
        return;
    }
    let mut intent: Option<(u32, Intent)> = None;
    egui::ScrollArea::horizontal()
        .id_salt("macro-strip")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for card in &cards {
                    let reach = Reach {
                        scene: scene.as_deref_mut(),
                        meta: meta.as_deref_mut(),
                    };
                    if let Some(chosen) = cell(ui, theme, card, reach, remote, panes, actions) {
                        intent = Some((card.found.id, chosen));
                    }
                }
            });
        });
    if let Some((id, chosen)) = intent {
        match chosen {
            Intent::Learn => {
                let target = Target::Macro { id };
                if remote.engine.is_learning_target(&target) {
                    remote.engine.cancel_learn();
                } else {
                    remote.engine.arm_new(target);
                }
            }
            Intent::Unpin => remote.engine.macros.set_pinned(id, false),
            Intent::Earlier => {
                remote.engine.macros.move_by(id, -1);
            }
            Intent::Later => {
                remote.engine.macros.move_by(id, 1);
            }
        }
    }
}

/// What a card's context menu asked for.
#[derive(Clone, Copy)]
enum Intent {
    Learn,
    Unpin,
    Earlier,
    Later,
}

/// The two banks, lent to one card at a time.
struct Reach<'a> {
    scene: Option<&'a mut ControlBank>,
    meta: Option<&'a mut ControlBank>,
}

/// The stage 1 strip: the playing scene's first controls.
fn legacy(ui: &mut Ui, theme: &Palette, bank: Option<&mut ControlBank>, panes: &mut Panes) {
    let gap = ui.spacing().item_spacing.x;
    let count = cells_that_fit(ui.available_width(), gap);
    let Some(bank) = bank else {
        placeholder(
            ui,
            theme,
            "Macros appear here: pin controls in the MAP pane, or the scene's first controls.",
        );
        return;
    };
    let picked = pick(bank, count);
    if picked.is_empty() {
        placeholder(ui, theme, "This scene has no controls to pin.");
        return;
    }
    ui.horizontal(|ui| {
        for index in picked {
            let Some(name) = bank.defs().get(index).map(|d| d.name.clone()) else {
                continue;
            };
            frame(ui, theme, None, |ui, width| {
                header(ui, theme, "SCENE", &name, None, width);
                control_body(ui, theme, bank, index, None, width, panes);
            });
        }
    });
}

fn placeholder(ui: &mut Ui, theme: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), STRIP_HEIGHT),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::TextStyle::Small.resolve(ui.style()),
        theme::rgb(theme.text_faint),
    );
}

/// A macro's identity colour `index` (below `strobe_io::macros::COLOURS`),
/// from the theme's series ramp, so it follows the theme (D210).
pub fn macro_colour(theme: &Palette, index: u8) -> crate::color::Rgb {
    theme.series(usize::from(index), usize::from(strobe_io::macros::COLOURS))
}

/// A card's frame; `body` draws inside it with the inner width. A macro
/// with a colour gets a band of it along the top.
fn frame(
    ui: &mut Ui,
    theme: &Palette,
    colour: Option<crate::color::Rgb>,
    body: impl FnOnce(&mut Ui, f32),
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(CELL_WIDTH, STRIP_HEIGHT), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.raised),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
    if let Some(colour) = colour {
        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 3.0));
        painter.rect_filled(
            band,
            CornerRadius {
                nw: theme::RADIUS,
                ne: theme::RADIUS,
                sw: 0,
                se: 0,
            },
            theme::rgb(colour),
        );
    }
    let inner = rect.shrink2(Vec2::new(SPACE_SM, SPACE_XS + 1.0));
    // A child that is *not* merged back into the row: a widget that is
    // wider than its cell must overflow into the clip, not push the next
    // cell along.
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(inner.expand(1.0));
    child.spacing_mut().item_spacing = Vec2::new(SPACE_XS, 2.0);
    // A row is as tall as its text here, not the window's interact height
    // (which density sets): the header, the value and the rail must fit
    // the card at every density, or the rail is clipped off its bottom.
    child.spacing_mut().interact_size.y = 14.0;
    body(&mut child, inner.width());
    response
}

/// The card's first line: a source badge, the name, and a tag on the right.
fn header(ui: &mut Ui, theme: &Palette, badge: &str, name: &str, tag: Option<&str>, width: f32) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(badge)
                .size(9.0)
                .strong()
                .color(theme::rgb(theme.accent)),
        );
        let room = width - 52.0 - tag.map_or(0.0, |t| t.len() as f32 * 6.0);
        let galley = widgets::truncated(
            ui,
            name,
            egui::TextStyle::Small.resolve(ui.style()),
            theme.text,
            room,
        );
        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        ui.painter()
            .galley(rect.min, galley, theme::rgb(theme.text));
        if let Some(tag) = tag {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(tag)
                        .size(9.0)
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
    });
}

/// One macro card. Returns what its context menu asked for.
fn cell(
    ui: &mut Ui,
    theme: &Palette,
    card: &Card,
    reach: Reach<'_>,
    remote: &Remote,
    panes: &mut Panes,
    actions: &mut Vec<Action>,
) -> Option<Intent> {
    let found = &card.found;
    let state = card.resolution.state;
    let usable = matches!(state, State::Resolved | State::Learning | State::Conflict);
    let flat = flatten(&found.target, &remote.engine.macros);
    let mut chosen = None;
    let learning = remote
        .engine
        .is_learning_target(&Target::Macro { id: found.id });
    let colour = found.color.map(|index| macro_colour(theme, index));
    let response = frame(ui, theme, colour, |ui, width| {
        let badge = match &flat {
            Ok(flat) => flat.target.badge(),
            Err(_) => found.target.badge(),
        };
        header(ui, theme, badge, &found.name, card.tag.as_deref(), width);
        if !usable {
            let (icon, colour) = map::state_look(theme, state);
            ui.horizontal(|ui| {
                crate::ui::icon::show(ui, icon, crate::ui::icon::DISCLOSURE, theme::rgb(colour));
                let sentence = match state {
                    State::Unavailable => "unavailable in current scene".to_string(),
                    State::Disabled => "disabled".to_string(),
                    State::Broken => "broken".to_string(),
                    State::Resolved | State::Learning | State::Conflict => String::new(),
                };
                ui.label(
                    egui::RichText::new(sentence)
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            });
            return;
        }
        if state == State::Learning || learning {
            ui.label(
                egui::RichText::new("move a control to bind it")
                    .small()
                    .color(theme::rgb(theme.accent)),
            );
            return;
        }
        if state == State::Conflict {
            ui.label(
                egui::RichText::new("conflict: source shared")
                    .small()
                    .color(theme::rgb(theme.warning)),
            );
            return;
        }
        let Ok(flat) = flat else { return };
        body(ui, theme, &flat, reach, remote, panes, width, actions);
    });
    response.context_menu(|ui| {
        if ui.button("Learn a MIDI or OSC source").clicked() {
            chosen = Some(Intent::Learn);
            ui.close();
        }
        if ui.button("Move earlier").clicked() {
            chosen = Some(Intent::Earlier);
            ui.close();
        }
        if ui.button("Move later").clicked() {
            chosen = Some(Intent::Later);
            ui.close();
        }
        if ui.button("Unpin").clicked() {
            chosen = Some(Intent::Unpin);
            ui.close();
        }
    });
    let reach = found.range.map(|[low, high]| {
        format!(
            "; a fader reaches {:.0}% to {:.0}% of the range",
            low * 100.0,
            high * 100.0
        )
    });
    if !card.tip.is_empty() || reach.is_some() {
        let driven = if card.tip.is_empty() {
            "No source yet".to_owned()
        } else {
            format!("Driven by {}", card.tip)
        };
        response.on_hover_text(format!("{driven}{}", reach.unwrap_or_default()));
    }
    chosen
}

/// The widget for a macro whose target is found.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the arguments are the card's whole context"
)]
fn body(
    ui: &mut Ui,
    theme: &Palette,
    flat: &Flat<'_>,
    reach: Reach<'_>,
    remote: &Remote,
    panes: &mut Panes,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let Reach { scene, meta } = reach;
    match flat.target {
        Target::Control { .. } | Target::Meta { .. } => {
            let Ok(found) = locate_control(flat, scene.as_deref(), meta.as_deref()) else {
                return;
            };
            let bank = match found.bank {
                strobe_io::target::BankId::Scene => scene,
                strobe_io::target::BankId::Meta => meta,
            };
            if let Some(bank) = bank {
                control_body(ui, theme, bank, found.index, found.component, width, panes);
            }
        }
        Target::Command { id } => {
            let label = crate::commands::CommandId::from_id(id)
                .map_or_else(|| id.clone(), |command| command.name().to_string());
            if controls::bang_button(ui, theme, &label).clicked() {
                actions.push(Action::RunCommand(id.clone()));
            }
        }
        Target::Cue { number } => {
            if controls::bang_button(ui, theme, &format!("Jump to cue {number}")).clicked() {
                actions.push(Action::JumpToCue(number.saturating_sub(1) as usize));
            }
        }
        Target::FavSlot { slot } => {
            if controls::bang_button(ui, theme, &format!("Recall fav {slot}")).clicked() {
                actions.push(Action::RecallFav(usize::from(*slot).saturating_sub(1)));
            }
        }
        Target::Preset { name } => {
            if controls::bang_button(ui, theme, &format!("Recall {name}")).clicked() {
                actions.push(Action::RecallPreset(name.clone()));
            }
        }
        Target::Macro { .. } => {}
    }
    let _ = remote;
}

/// The widget for one control, sized for a card. `component` picks one
/// channel of a pad or colour.
fn control_body(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    component: Option<usize>,
    width: f32,
    panes: &mut Panes,
) {
    let Some(def) = bank.defs().get(index).cloned() else {
        return;
    };
    let target = bank.target(index).copied().unwrap_or_default();
    match (def.kind, component) {
        (ControlKind::Slider(_) | ControlKind::Knob(_), _)
        | (ControlKind::Xy(_) | ControlKind::Color { .. }, Some(_)) => {
            let c = component.unwrap_or(0);
            let (min, max, default) = (def.min[c], def.max[c], def.default[c]);
            let mut value = if target[c].is_finite() {
                target[c]
            } else {
                default
            };
            let places = controls::decimals(min, max);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{value:.places$}"))
                        .monospace()
                        .size(11.0)
                        .color(theme::rgb(theme.text_muted)),
                );
            });
            let (_, out) = controls::rail(
                ui,
                theme,
                ("macro", index, c),
                width,
                &mut value,
                min,
                max,
                default,
            );
            if out.reset {
                bank.set_default(index);
            } else if out.changed {
                bank.set(index, Some(c), &[value]);
            }
        }
        (ControlKind::Toggle { .. }, _) => {
            let on = def.normalize(0, target[0]) >= 0.5;
            let label = if on { "ON" } else { "OFF" };
            if widgets::tool_button(ui, theme, Icon::Check, label, on, "Toggle").clicked() {
                let next = if on { def.min[0] } else { def.max[0] };
                bank.set(index, Some(0), &[next]);
            }
        }
        (ControlKind::Bang(_), _) => {
            if controls::bang_button(ui, theme, "Trigger").clicked() {
                bank.press(index);
            }
        }
        (ControlKind::Dropdown { .. }, _) => {
            let selected =
                (target[0].round().max(0.0) as usize).min(def.labels.len().saturating_sub(1));
            let current = def.labels.get(selected).cloned().unwrap_or_default();
            egui::ComboBox::from_id_salt(("macro-dropdown", index))
                .width(width - 4.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (option, label) in def.labels.iter().enumerate() {
                        if ui.selectable_label(option == selected, label).clicked() {
                            bank.set(index, Some(0), &[option as f32]);
                        }
                    }
                });
        }
        (ControlKind::Color { .. }, None) => {
            let mut rgb = target;
            if ui.color_edit_button_rgb(&mut rgb).changed() {
                bank.set(index, None, &rgb);
            }
        }
        (ControlKind::Xy(_), None) => {
            if widgets::tool_button(
                ui,
                theme,
                Icon::Controls,
                "XY pad",
                false,
                "Open the controls pane: an XY pad needs more room than a macro cell",
            )
            .clicked()
            {
                panes.open(Pane::Inspector);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use strobe_scene::scene::SceneDef;

    fn bank(count: usize) -> Result<ControlBank> {
        let controls: Vec<String> = (0..count)
            .map(|i| format!(r#"{{ "NAME": "c{i}", "TYPE": "slider" }}"#))
            .collect();
        let json = format!(r#"{{ "CONTROLS": [{}] }}"#, controls.join(","));
        Ok(ControlBank::new(SceneDef::parse(&json, "t")?.controls)?)
    }

    #[test]
    fn the_fallback_strip_shows_the_first_controls_in_declaration_order() -> Result<()> {
        let bank = bank(10)?;
        assert_eq!(pick(&bank, 4), vec![0, 1, 2, 3], "first four");
        assert_eq!(
            pick(&bank, 99),
            (0..10).collect::<Vec<_>>(),
            "all there are"
        );
        assert!(pick(&bank, 0).is_empty(), "none asked for");
        Ok(())
    }

    #[test]
    fn a_wider_window_shows_more_fallback_cells_up_to_the_cap() {
        let gap = 8.0;
        let at = |w: f32| cells_that_fit(w, gap);
        assert_eq!(at(0.0), 1, "never zero cells");
        assert_eq!(at(f32::NAN), 1, "nonsense width");
        assert!(at(500.0) < at(900.0), "more room, more cells");
        assert_eq!(at(10_000.0), MAX_MACROS, "capped");
        assert!(
            at(1280.0 - 60.0 - 300.0) >= 4,
            "1280 with the inspector open still shows four cells"
        );
    }
}
