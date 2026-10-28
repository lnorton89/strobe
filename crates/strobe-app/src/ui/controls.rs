// ============================================================================
// Module:       strobe_app::ui::controls
// Description:  One widget per scene control kind, written against the
//               control bank: the slider rail, toggle, bang, XY pad, colour
//               swatch and dropdown, and the grouping of controls by
//               UI_GROUP.
//
// Dependencies: egui; strobe-scene (ControlBank, ControlDef, ControlKind);
//               super::{theme, widgets, icon}
// ============================================================================

//! The control widgets (UI_DESIGN section 10).
//!
//! One widget per [`ControlKind`], chosen by an exhaustive match so a new
//! control type cannot silently render nothing:
//!
//! | Control | Widget | Writes |
//! |---|---|---|
//! | slider, knob | [`rail`] over the control's real range, plus an exact numeric field | `set` |
//! | toggle | a switch | `set` to `MIN` or `MAX` |
//! | bang, bang counter | a button with a pressed state | `press` |
//! | xy | a drag pad with keyboard nudging | `set` of both components |
//! | color | a colour swatch with a popup picker | `set` of three components |
//! | dropdown | a combo box of labels | `set` to the option index |
//!
//! Widgets show the control's *target* (what was asked for), not its
//! smoothed output, so a smooth slider does not fight the hand moving it.
//!
//! ## The slider's interaction contract
//!
//! These preserve Synesthesia's established modifier conventions
//! (UI_DESIGN section 10, UI_IMPLEMENTATION_PLAN section 10):
//!
//! - click or drag the rail to set the value (relative, fine, with Shift:
//!   a tenth of the speed);
//! - double-click the rail, or middle-click it, to reset to the default;
//! - the reset button beside the value does the same, and appears only when
//!   the value differs from the default, so it is also the "modified" mark;
//! - click the number to type an exact value;
//! - with the rail focused, arrows move by 1% of the range (Shift: 0.1%,
//!   Ctrl: 10%).
//!
//! Every value is checked for being finite before it is drawn or written.

use super::icon as icons;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::palette::Palette;
use egui::{CornerRadius, Key, Modifiers, Rect, Response, Sense, Stroke, Ui, Vec2};
use strobe_scene::control::{Bang, ControlBank, ControlDef, ControlKind};

/// Size of the xy pad, in points.
const PAD_SIZE: Vec2 = Vec2::new(160.0, 108.0);

/// The height of a slider rail's hit area.
const RAIL_HEIGHT: f32 = 18.0;

/// How large the rows are drawn (D291): the side panel's compact rows, or
/// the main view's Controls page, sized for a finger as much as a mouse.
/// The widgets and what they write are the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// The side panel's Controls tab.
    Panel,
    /// The main view's Controls page: taller rails, a larger pad, wider
    /// switches and buttons.
    Touch,
}

impl Scale {
    /// A slider rail's hit height.
    const fn rail_height(self) -> f32 {
        match self {
            Self::Panel => RAIL_HEIGHT,
            Self::Touch => 32.0,
        }
    }

    /// The xy pad, at most `width` wide.
    fn pad_size(self, width: f32) -> Vec2 {
        match self {
            Self::Panel => PAD_SIZE,
            Self::Touch => {
                let w = width.clamp(PAD_SIZE.x, 280.0);
                Vec2::new(w, w * 2.0 / 3.0)
            }
        }
    }

    /// A bang button's height.
    const fn bang_height(self) -> f32 {
        match self {
            Self::Panel => 26.0,
            Self::Touch => 40.0,
        }
    }

    /// A toggle switch's size.
    const fn switch_size(self) -> Vec2 {
        match self {
            Self::Panel => Vec2::new(32.0, 18.0),
            Self::Touch => Vec2::new(48.0, 26.0),
        }
    }

    /// A colour control's swatch.
    const fn swatch_size(self) -> Vec2 {
        match self {
            Self::Panel => Vec2::new(40.0, 20.0),
            Self::Touch => Vec2::new(56.0, 30.0),
        }
    }

    /// The gap under each row, in steps of `SPACE_XS`.
    const fn gap_steps(self) -> f32 {
        match self {
            Self::Panel => 1.0,
            Self::Touch => 2.0,
        }
    }
}

/// How much of the range one arrow press moves the value.
const NUDGE: f32 = 0.01;

/// Control indices bucketed by `UI_GROUP`: the ungrouped bucket first, then
/// groups in the order they first appear.
pub fn group_order(defs: &[ControlDef]) -> Vec<(Option<String>, Vec<usize>)> {
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

/// The value after an arrow press or a drag, kept inside the range.
fn clamp_to(value: f32, min: f32, max: f32) -> f32 {
    if !value.is_finite() {
        return min;
    }
    value.clamp(min.min(max), min.max(max))
}

/// The fraction of the way along `min..max` that `value` is, 0..=1; zero for
/// a degenerate range or a non-finite value.
pub fn fraction(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if !value.is_finite() || !span.is_finite() || span.abs() < f32::EPSILON {
        return 0.0;
    }
    ((value - min) / span).clamp(0.0, 1.0)
}

/// What a slider rail did this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RailOut {
    /// The value changed.
    pub changed: bool,
    /// The operator asked for the default (double or middle click).
    pub reset: bool,
}

/// A horizontal slider rail. See the module docs for the interaction
/// contract. `id_salt` must identify the control, not its position.
#[expect(
    clippy::too_many_arguments,
    reason = "a rail is defined by exactly these; grouping them would only rename them"
)]
pub fn rail(
    ui: &mut Ui,
    theme: &Palette,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
) -> (Response, RailOut) {
    rail_at(
        ui,
        theme,
        id_salt,
        width,
        RAIL_HEIGHT,
        value,
        (min, max, default),
    )
}

/// [`rail`] with its hit height: `range` is the minimum, the maximum and
/// the default.
fn rail_at(
    ui: &mut Ui,
    theme: &Palette,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    height: f32,
    value: &mut f32,
    range: (f32, f32, f32),
) -> (Response, RailOut) {
    let (min, max, default) = range;
    // The rail's identity comes from `id_salt`, not from where it was laid
    // out, so a drag in progress stays with its control when the rows above
    // it change (a group collapsing, a control appearing).
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width.max(24.0), height), Sense::hover());
    let response = ui
        .interact(
            rect,
            ui.scope_id().with(("rail", id_salt)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let mut out = RailOut::default();
    let span = max - min;
    let usable = span.is_finite() && span.abs() > f32::EPSILON;

    if usable {
        let before = *value;
        let shift = ui.input(|i| i.modifiers.shift);
        if response.double_clicked() || response.middle_clicked() {
            *value = default;
            out.reset = true;
        } else if (response.drag_started() || response.clicked()) && !shift {
            if let Some(pos) = response.interact_pointer_pos() {
                let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                *value = min + t * span;
            }
        } else if response.dragged() {
            let delta = response.drag_delta().x / rect.width() * span;
            let delta = if shift { delta * 0.1 } else { delta };
            if !shift && let Some(pos) = response.interact_pointer_pos() {
                let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                *value = min + t * span;
            } else {
                *value += delta;
            }
        }
        if response.has_focus() {
            let (step, modifiers) = ui.input(|i| {
                let mut step = 0.0_f32;
                for key in [Key::ArrowLeft, Key::ArrowDown] {
                    if i.key_pressed(key) {
                        step -= 1.0;
                    }
                }
                for key in [Key::ArrowRight, Key::ArrowUp] {
                    if i.key_pressed(key) {
                        step += 1.0;
                    }
                }
                (step, i.modifiers)
            });
            if step != 0.0 {
                *value += step * nudge_size(modifiers) * span;
            }
        }
        *value = clamp_to(*value, min, max);
        if (*value - before).abs() > f32::EPSILON || out.reset {
            out.changed = true;
        }
    }

    let t = fraction(*value, min, max);
    let painter = ui.painter();
    // A taller rail (the Controls page, D291) gets a thicker track and a
    // larger handle, so a finger can see what it holds.
    let big = height > RAIL_HEIGHT;
    let track = Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width(), if big { 6.0 } else { 4.0 }),
    );
    let radius = CornerRadius::same(2);
    painter.rect_filled(track, radius, theme::rgb(theme.control));
    if usable {
        let filled = Rect::from_min_max(
            track.min,
            egui::pos2(track.left() + track.width() * t, track.bottom()),
        );
        painter.rect_filled(filled, radius, theme::rgb(theme.accent));
        // The default's tick: where "reset" goes.
        let tick = fraction(default, min, max);
        if (tick - t).abs() > 0.02 {
            let x = track.left() + track.width() * tick;
            painter.vline(
                x,
                rect.center().y - 4.0..=rect.center().y + 4.0,
                Stroke::new(1.0, theme::rgb(theme.text_faint)),
            );
        }
        let lit = response.hovered() || response.dragged() || response.has_focus();
        let handle = egui::pos2(track.left() + track.width() * t, rect.center().y);
        let grow = if big { 3.0 } else { 0.0 };
        painter.circle_filled(
            handle,
            grow + if lit { 7.0 } else { 6.0 },
            theme::rgb(if lit { theme.accent_hover } else { theme.text }),
        );
        if response.has_focus() {
            painter.circle_stroke(
                handle,
                grow + 9.0,
                Stroke::new(1.5, theme::rgb(theme.accent)),
            );
        }
    }
    (response, out)
}

/// How far one arrow press moves a slider, as a fraction of its range.
fn nudge_size(modifiers: Modifiers) -> f32 {
    if modifiers.command {
        NUDGE * 10.0
    } else if modifiers.shift {
        NUDGE * 0.1
    } else {
        NUDGE
    }
}

/// Decimal places worth showing for a range: more for a small one.
pub fn decimals(min: f32, max: f32) -> usize {
    let span = (max - min).abs();
    if !span.is_finite() || span >= 100.0 {
        0
    } else if span >= 10.0 {
        1
    } else if span >= 1.0 {
        2
    } else {
        3
    }
}

/// Pinning a control as a Performance Macro from its row (D210): the
/// name's context menu offers it.
pub struct Pins<'a> {
    /// The bank's controls that already have a pinned macro, by name.
    pub pinned: &'a [String],
    /// What was chosen this frame: a control's name, and pin (`true`) or
    /// unpin.
    pub chosen: Option<(String, bool)>,
}

/// Gives `response` (a control's name or button) the pin menu.
fn pin_menu(response: &Response, def: &ControlDef, pins: &mut Option<&mut Pins<'_>>) {
    let Some(pins) = pins.as_deref_mut() else {
        return;
    };
    let pinned = pins.pinned.contains(&def.name);
    response.context_menu(|ui| {
        let (label, pin) = if pinned {
            ("Unpin the macro", false)
        } else {
            ("Pin as a macro", true)
        };
        if ui.button(label).clicked() {
            pins.chosen = Some((def.name.clone(), pin));
            ui.close();
        }
    });
}

/// A control's name as a person reads it (UI review S6): underscores as
/// spaces and the first letter capital, so `vertical_mirror` shows
/// "Vertical mirror" and `master` "Master". Display only: scripts,
/// mappings and presets keep the name as written.
pub fn display_name(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spaced,
    }
}

/// A control's name, which right-clicks to the pin menu.
fn name_label(
    ui: &mut Ui,
    theme: &Palette,
    def: &ControlDef,
    pins: &mut Option<&mut Pins<'_>>,
) -> Response {
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(display_name(&def.name)).color(theme::rgb(theme.text)),
        )
        .sense(Sense::click()),
    );
    pin_menu(&response, def, pins);
    response
}

/// Draws every control in `members` (indices into the bank). Group headers
/// are the inspector's business; this draws the rows, at `scale`. With
/// `pins`, a control's name offers to pin it as a macro.
pub fn rows(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    members: &[usize],
    mut pins: Option<&mut Pins<'_>>,
    scale: Scale,
) {
    let pins = &mut pins;
    for &index in members {
        let Some(def) = bank.defs().get(index).cloned() else {
            continue;
        };
        let target = bank.target(index).copied().unwrap_or_default();
        let output = bank.output(index).copied().unwrap_or_default();
        let drawn = ui.push_id(index, |ui| match def.kind {
            ControlKind::Slider(_) | ControlKind::Knob(_) => {
                slider_row(ui, theme, bank, index, &def, target[0], pins, scale);
            }
            ControlKind::Toggle { .. } => {
                toggle_row(ui, theme, bank, index, &def, target[0], pins, scale);
            }
            ControlKind::Bang(kind) => {
                bang_row(ui, theme, bank, index, &def, (kind, output[0]), pins, scale);
            }
            ControlKind::Xy(_) => xy_row(ui, theme, bank, index, &def, target, pins, scale),
            ControlKind::Color { .. } => {
                color_row(ui, theme, bank, index, &def, target, pins, scale);
            }
            ControlKind::Dropdown { .. } => {
                dropdown_row(ui, theme, bank, index, &def, target[0], pins);
            }
        });
        // A controlling Strobe draws its own copy of the row here (D515).
        crate::remote_controls::noted(
            ui,
            bank,
            index,
            drawn.response.rect,
            matches!(scale, Scale::Touch),
        );
        ui.add_space(SPACE_XS * scale.gap_steps());
    }
}

fn describe(response: Response, def: &ControlDef) -> Response {
    match &def.description {
        Some(text) => response.on_hover_text(text),
        None => response,
    }
}

/// The small reset button that appears beside a modified control.
fn reset_button(ui: &mut Ui, theme: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
    let fill = widgets::hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.panel,
        theme.hover,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(icons::DISCLOSURE)),
        Icon::Reset,
        theme::rgb(theme.text_muted),
    );
    response.on_hover_text("Reset to the default")
}

#[expect(
    clippy::too_many_arguments,
    reason = "a row's whole context, as its siblings take"
)]
fn slider_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: f32,
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    let (min, max, default) = (def.min[0], def.max[0], def.default[0]);
    if (max - min).abs() < f32::EPSILON {
        ui.label(format!("{}: {v:.3}", def.name));
        return;
    }
    let mut value = if v.is_finite() { v } else { default };
    let places = decimals(min, max);
    let modified = (value - default).abs() > (max - min).abs() * 0.001;
    ui.horizontal(|ui| {
        let label = name_label(ui, theme, def, pins);
        describe(label, def);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The number at the row's right edge, over the rail's end; the
            // reset button left of it, its slot kept while it is hidden, so
            // nothing moves when a control is modified. With the slot at the
            // edge the number stood a button's width short of the rail.
            let drag = ui.add(
                egui::DragValue::new(&mut value)
                    .range(min.min(max)..=min.max(max))
                    .speed((max - min).abs() / 400.0)
                    .min_decimals(places)
                    .max_decimals(places)
                    .update_while_editing(false),
            );
            if drag.changed() {
                bank.set(index, Some(0), &[clamp_to(value, min, max)]);
            }
            if modified {
                if reset_button(ui, theme).clicked() {
                    bank.set_default(index);
                }
            } else {
                ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            }
        });
    });
    let width = ui.available_width();
    let (_, out) = rail_at(
        ui,
        theme,
        index,
        width,
        scale.rail_height(),
        &mut value,
        (min, max, default),
    );
    if out.reset {
        bank.set_default(index);
    } else if out.changed {
        bank.set(index, Some(0), &[value]);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a row's whole context, as its siblings take"
)]
fn toggle_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: f32,
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    let on = def.normalize(0, v) >= 0.5;
    let response = ui
        .horizontal(|ui| {
            let size = scale.switch_size();
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let t = super::motion::toggle(ui.ctx(), response.id, on, super::motion::INSTANT);
            let painter = ui.painter();
            let track = if on { theme.accent } else { theme.control };
            let half = size.y / 2.0;
            let round = CornerRadius::same(half as u8);
            painter.rect_filled(rect, round, theme::rgb(track));
            let x = rect.left() + half + t * (rect.width() - size.y);
            painter.circle_filled(
                egui::pos2(x, rect.center().y),
                half - 2.5,
                theme::rgb(if on { theme.text_on_accent } else { theme.text }),
            );
            if response.has_focus() {
                painter.rect_stroke(
                    rect.expand(2.0),
                    CornerRadius::same(half as u8 + 1),
                    Stroke::new(1.5, theme::rgb(theme.accent)),
                    egui::StrokeKind::Outside,
                );
            }
            name_label(ui, theme, def, pins);
            ui.label(
                egui::RichText::new(if on { "ON" } else { "OFF" })
                    .small()
                    .color(theme::rgb(if on { theme.accent } else { theme.text_muted })),
            );
            response
        })
        .inner;
    if describe(response, def).clicked() {
        let next = if on { def.min[0] } else { def.max[0] };
        bank.set(index, Some(0), &[next]);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; a row's whole context, as its siblings take"
)]
fn bang_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    (kind, output): (Bang, f32),
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    ui.horizontal(|ui| {
        let response = bang_button_sized(ui, theme, &def.name, scale.bang_height());
        pin_menu(&response, def, pins);
        if describe(response, def).clicked() {
            bank.press(index);
        }
        match kind {
            Bang::Counter | Bang::CounterSmooth => {
                ui.label(
                    egui::RichText::new(format!("{output:.0}"))
                        .monospace()
                        .color(theme::rgb(theme.text_muted)),
                );
            }
            Bang::Plain | Bang::Smooth => {}
        }
    });
}

/// A momentary button with a visible pressed state.
pub fn bang_button(ui: &mut Ui, theme: &Palette, name: &str) -> Response {
    bang_button_sized(ui, theme, name, Scale::Panel.bang_height())
}

/// [`bang_button`], `height` tall.
fn bang_button_sized(ui: &mut Ui, theme: &Palette, name: &str, height: f32) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), font, theme::rgb(theme.text));
    let size = Vec2::new(galley.size().x + SPACE_SM * 3.0, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed { theme.accent } else { theme.raised };
    let text = if pressed {
        theme.text_on_accent
    } else {
        theme.text
    };
    let fill = if pressed {
        theme::rgb(fill)
    } else {
        widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            theme.raised,
            theme.hover,
        )
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(
            1.0,
            theme::rgb(if pressed { theme.accent } else { theme.border }),
        ),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(text),
    );
    response
}

#[expect(
    clippy::too_many_arguments,
    reason = "a row's whole context, as its siblings take"
)]
fn xy_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: [f32; 3],
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    let modified = def.default[0] != v[0] || def.default[1] != v[1];
    ui.horizontal(|ui| {
        name_label(ui, theme, def, pins);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The readout at the edge, the reset left of it: it no longer
            // moves when the pad leaves its default.
            ui.label(
                egui::RichText::new(format!("{:.2}, {:.2}", v[0], v[1]))
                    .monospace()
                    .color(theme::rgb(theme.text_muted)),
            );
            if modified && reset_button(ui, theme).clicked() {
                bank.set_default(index);
            }
        });
    });
    let pad = scale.pad_size(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(pad, Sense::click_and_drag());
    let response = describe(response, def);
    if let Some(pos) = response.interact_pointer_pos() {
        let tx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let ty = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        bank.set(
            index,
            None,
            &[def.denormalize(0, tx), def.denormalize(1, ty)],
        );
    } else if response.has_focus() {
        // Keyboard nudging: arrows move the marker by 1% (Shift 0.1%, Ctrl 10%).
        let (dx, dy, modifiers) = ui.input(|i| {
            let axis = |plus: Key, minus: Key| {
                f32::from(i.key_pressed(plus)) - f32::from(i.key_pressed(minus))
            };
            (
                axis(Key::ArrowRight, Key::ArrowLeft),
                axis(Key::ArrowUp, Key::ArrowDown),
                i.modifiers,
            )
        });
        if dx != 0.0 || dy != 0.0 {
            let step = nudge_size(modifiers);
            let x = (def.normalize(0, v[0]) + dx * step).clamp(0.0, 1.0);
            let y = (def.normalize(1, v[1]) + dy * step).clamp(0.0, 1.0);
            bank.set(index, None, &[def.denormalize(0, x), def.denormalize(1, y)]);
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.app),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        Stroke::new(
            1.0,
            theme::rgb(if response.has_focus() {
                theme.accent
            } else {
                theme.border
            }),
        ),
        egui::StrokeKind::Inside,
    );
    let at = egui::pos2(
        rect.left() + def.normalize(0, v[0]) * rect.width(),
        rect.bottom() - def.normalize(1, v[1]) * rect.height(),
    );
    let guide = Stroke::new(1.0, theme::rgb(theme.grid));
    painter.hline(rect.x_range(), at.y, guide);
    painter.vline(at.x, rect.y_range(), guide);
    painter.circle_filled(at, 5.0, theme::rgb(theme.accent));
}

#[expect(
    clippy::too_many_arguments,
    reason = "a row's whole context, as its siblings take"
)]
fn color_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: [f32; 3],
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    ui.horizontal(|ui| {
        let mut rgb = v;
        let response = swatch(ui, theme, &mut rgb, scale.swatch_size());
        if describe(response, def).changed() {
            bank.set(index, None, &rgb);
        }
        name_label(ui, theme, def, pins);
    });
}

/// A colour swatch, `size` large, that opens egui's picker.
///
/// egui sizes its swatch by `interact_size`, whose width the theme sets to
/// zero so buttons size to their words: left alone, the swatch had no width
/// and could not be clicked, in the panel and the macro strip alike. The
/// size is set in a scope of its own. The swatch is outlined, so a black or
/// panel-coloured value still reads as a swatch on the dark theme.
pub fn swatch(ui: &mut Ui, theme: &Palette, rgb: &mut [f32; 3], size: Vec2) -> Response {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = size;
            ui.color_edit_button_rgb(rgb)
        })
        .inner;
    ui.painter().rect_stroke(
        response.rect,
        CornerRadius::same(2),
        Stroke::new(1.0, theme::rgb(theme.border)),
        egui::StrokeKind::Outside,
    );
    response
}

fn dropdown_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: f32,
    pins: &mut Option<&mut Pins<'_>>,
) {
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
        name_label(ui, theme, def, pins);
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

    #[test]
    fn a_fraction_is_zero_for_nonsense_and_clamped_otherwise() {
        assert_eq!(fraction(f32::NAN, 0.0, 1.0), 0.0, "NaN value");
        assert_eq!(fraction(0.5, 1.0, 1.0), 0.0, "zero-width range");
        assert_eq!(fraction(0.5, 0.0, f32::INFINITY), 0.0, "infinite range");
        assert_eq!(fraction(5.0, 0.0, 1.0), 1.0, "over the top");
        assert!((fraction(0.25, 0.0, 1.0) - 0.25).abs() < 1e-6, "inside");
        assert!(
            (fraction(-1.0, -2.0, 2.0) - 0.25).abs() < 1e-6,
            "negative range"
        );
    }

    #[test]
    fn a_value_is_clamped_into_a_reversed_range_too() {
        assert_eq!(clamp_to(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp_to(-5.0, 1.0, 0.0), 0.0, "min and max swapped");
        assert_eq!(
            clamp_to(f32::NAN, 0.25, 1.0),
            0.25,
            "NaN falls to the minimum"
        );
    }

    #[test]
    fn shift_is_finer_and_ctrl_is_coarser_than_a_plain_nudge() {
        let plain = nudge_size(Modifiers::NONE);
        let fine = nudge_size(Modifiers::SHIFT);
        let coarse = nudge_size(Modifiers::COMMAND);
        assert!(fine < plain && plain < coarse, "{fine} {plain} {coarse}");
    }

    #[test]
    fn a_small_range_shows_more_decimals_than_a_large_one() {
        assert!(decimals(0.0, 1.0) > decimals(0.0, 10.0));
        assert!(decimals(0.0, 10.0) > decimals(0.0, 1000.0));
        assert_eq!(decimals(0.0, f32::NAN), 0, "nonsense shows an integer");
    }

    /// Runs one egui pass with `events` and reports what the rail did.
    fn run_rail(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        value: &mut f32,
        default: f32,
    ) -> (Rect, RailOut) {
        let theme = crate::palette::fallback().clone();
        let mut out = RailOut::default();
        let mut rect = Rect::NOTHING;
        let _ = crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(400.0, 200.0), events),
            |ui| {
                let (response, result) = rail(ui, &theme, 0_u8, 200.0, value, 0.0, 10.0, default);
                rect = response.rect;
                out = result;
            },
        );
        (rect, out)
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn clicking_the_rail_sets_the_value_from_the_pointer() {
        let ctx = egui::Context::default();
        let mut value = 0.0;
        let (rect, _) = run_rail(&ctx, Vec::new(), &mut value, 5.0);
        let at = egui::pos2(rect.left() + rect.width() * 0.75, rect.center().y);
        let _ = run_rail(
            &ctx,
            vec![egui::Event::PointerMoved(at), press(at, true)],
            &mut value,
            5.0,
        );
        let (_, out) = run_rail(&ctx, vec![press(at, false)], &mut value, 5.0);
        assert!(
            (value - 7.5).abs() < 0.2,
            "a click at 75% set {value} (changed this frame: {})",
            out.changed
        );
    }

    #[test]
    fn a_double_click_resets_to_the_default() {
        let ctx = egui::Context::default();
        let mut value = 9.0;
        let (rect, _) = run_rail(&ctx, Vec::new(), &mut value, 2.0);
        let at = rect.center();
        let click = |ctx: &egui::Context, value: &mut f32| {
            let _ = run_rail(
                ctx,
                vec![egui::Event::PointerMoved(at), press(at, true)],
                value,
                2.0,
            );
            run_rail(ctx, vec![press(at, false)], value, 2.0).1
        };
        let _ = click(&ctx, &mut value);
        let out = click(&ctx, &mut value);
        assert!(out.reset, "a second click in a row is a double click");
        assert!(
            (value - 2.0).abs() < f32::EPSILON,
            "reset to the default: {value}"
        );
    }

    #[test]
    fn a_non_finite_value_is_repaired_not_propagated() {
        let ctx = egui::Context::default();
        let mut value = f32::NAN;
        let (rect, _) = run_rail(&ctx, Vec::new(), &mut value, 1.0);
        let at = egui::pos2(rect.left() + 10.0, rect.center().y);
        let _ = run_rail(
            &ctx,
            vec![egui::Event::PointerMoved(at), press(at, true)],
            &mut value,
            1.0,
        );
        assert!(value.is_finite(), "the rail left {value} in a control");
    }

    #[test]
    fn a_control_name_reads_as_words_and_keeps_its_letters() {
        assert_eq!(
            display_name("vertical_mirror"),
            "Vertical mirror",
            "underscores"
        );
        assert_eq!(display_name("master"), "Master", "a capital");
        assert_eq!(display_name("hueShift"), "HueShift", "letters kept");
        assert_eq!(display_name(""), "", "empty");
    }
}
