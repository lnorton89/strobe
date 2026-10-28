// ============================================================================
// Module:       strobe_app::ui::controls
// Description:  One widget per scene control kind, written against the
//               control bank: the value bar, the toggle chip, bang, XY pad,
//               colour row and dropdown, the macro cards' slider rail, and
//               the grouping of controls by UI_GROUP.
//
// Dependencies: egui; strobe-scene (ControlBank, ControlDef, ControlKind,
//               Mode); super::{theme, widgets, icon, fonts, motion}
// ============================================================================

//! The control widgets (UI_DESIGN section 10).
//!
//! One widget per [`ControlKind`], chosen by an exhaustive match so a new
//! control type cannot silently render nothing:
//!
//! | Control | Widget | Writes |
//! |---|---|---|
//! | slider, knob | a [`bar`] over the control's real range | `set` |
//! | toggle | a chip with a lit dot, the row's height | `set` to `MIN` or `MAX` |
//! | bang, bang counter | a button with a pressed state | `press` |
//! | xy | a drag pad with keyboard nudging | `set` of both components |
//! | color | a row with the hex value and a swatch that opens a picker | `set` of three components |
//! | dropdown | a combo box of labels | `set` to the option index |
//!
//! Widgets show the control's *target* (what was asked for), so a smooth
//! slider does not fight the hand moving it. The bar also draws the
//! smoothed *output* as a thin line while it is still easing there.
//!
//! ## The value bar (D510, D511)
//!
//! A float control is one row: the row is the slider, filled to the value,
//! with the name inside it on the left and the number on the right
//! (Blender's and Tweakpane's number fields, lil-gui's sliders). About a
//! third of the height of a name over a rail, and the fill reads before
//! the number does. What it shows:
//!
//! - the fill, from the left, or from zero when the range spans it (a
//!   bipolar control reads "nothing" in its middle);
//! - a 2-point edge at the value, in the accent;
//! - a notch at the top and bottom where the default is, while the value
//!   is elsewhere, and the number brighter and semibold: what was changed;
//! - a thin line at the smoothed output while it catches up (not for a
//!   speed control, whose output is a running phase; its number says
//!   `/s` instead);
//! - `MACRO` before the number when the control is pinned (D210).
//!
//! The interaction contract (D511):
//!
//! - drag anywhere on the row: relative, so taking hold of a control never
//!   makes the picture jump; Shift drags a tenth as fast;
//! - click (press and release without moving) to type an exact value:
//!   Enter or a click away sets it, Esc cancels. The field opens once the
//!   double-click window has passed, so a double click never opens it;
//! - double-click or middle-click to reset to the default, as Synesthesia
//!   does; the right-click menu says Reset too;
//! - with the row focused, arrows move by 1% of the range (Shift: 0.1%,
//!   Ctrl: 10%).
//!
//! The macro cards keep the [`rail`]: a click there still sets the value
//! where it lands.
//!
//! Every value is checked for being finite before it is drawn or written.

use super::icon as icons;
use super::theme::{self, Density, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::palette::Palette;
use egui::{CornerRadius, Key, Modifiers, Rect, Response, Sense, Stroke, Ui, Vec2};
use strobe_scene::control::{Bang, ControlBank, ControlDef, ControlKind, Mode};

/// Size of the xy pad, in points.
const PAD_SIZE: Vec2 = Vec2::new(160.0, 108.0);

/// The height of a slider rail's hit area.
const RAIL_HEIGHT: f32 = 18.0;

/// A value bar's and a chip's corner radius: a step under the buttons', so
/// a column of rows reads as rows.
const BAR_RADIUS: u8 = 4;

/// How long a click on a value bar waits before it opens the number for
/// typing: egui's double-click window, so the first click of a double
/// click (reset) never opens the field.
const TYPE_AFTER: f64 = 0.3;

/// The width of the field a value is typed into.
const TYPE_WIDTH: f32 = 72.0;

/// How large the rows are drawn (D291): the side panel's compact rows, or
/// the main view's Controls page, sized for a finger as much as a mouse.
/// The widgets and what they write are the same either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// The side panel's Controls tab.
    Panel,
    /// The main view's Controls page: taller rows, a larger pad and
    /// buttons.
    Touch,
}

impl Scale {
    /// A value bar's, chip's and colour row's height at `density` (D510):
    /// the window's density setting packs them as it packs every list.
    pub const fn bar_height(self, density: Density) -> f32 {
        match (self, density) {
            (Self::Panel, Density::Comfortable) => 24.0,
            (Self::Panel, Density::Compact) => 20.0,
            (Self::Touch, Density::Comfortable) => 30.0,
            (Self::Touch, Density::Compact) => 24.0,
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

/// One float control drawn as a value bar (D510): what the row shows
/// besides the value it changes.
pub struct Bar<'a> {
    /// The control's name as a person reads it.
    pub label: &'a str,
    /// The value at the bar's left end.
    pub min: f32,
    /// The value at its right end.
    pub max: f32,
    /// Where reset goes; notched while the value is elsewhere.
    pub default: f32,
    /// The control's smoothed output, drawn as a thin line while it eases
    /// toward the value. `None` where the output is not a value on this
    /// range (a speed control's is a running phase).
    pub output: Option<f32>,
    /// Said after the number: `" /s"` for a rate.
    pub unit: &'static str,
    /// A short mark before the number (a pinned macro).
    pub tag: Option<&'a str>,
    /// The row's height.
    pub height: f32,
}

/// Where a bar's fill starts, as a fraction of its width: zero's place when
/// the range spans zero (a bipolar control reads "nothing" in its middle),
/// the left end otherwise.
pub fn fill_origin(min: f32, max: f32) -> f32 {
    if min.min(max) < 0.0 && min.max(max) > 0.0 {
        fraction(0.0, min, max)
    } else {
        0.0
    }
}

/// A value bar: the whole row is the slider. See the module docs for what
/// it draws and its interaction contract. `id_salt` must identify the
/// control, not its position.
pub fn bar(
    ui: &mut Ui,
    theme: &Palette,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    spec: &Bar<'_>,
    value: &mut f32,
) -> (Response, RailOut) {
    let (min, max, default) = (spec.min, spec.max, spec.default);
    let width = ui.available_width().max(48.0);
    // The identity comes from `id_salt`, as the rail's does, so a drag or a
    // typed value stays with its control when the rows above it change.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, spec.height), Sense::hover());
    let response = ui
        .interact(
            rect,
            ui.scope_id().with(("bar", id_salt)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let typing_id = response.id.with("typing");
    let after_id = response.id.with("type-after");
    let typing = ui.data(|d| d.get_temp::<String>(typing_id));
    let mut out = RailOut::default();
    let span = max - min;
    let usable = span.is_finite() && span.abs() > f32::EPSILON;
    let places = decimals(min, max);

    if usable && typing.is_none() {
        let before = *value;
        let now = ui.input(|i| i.time);
        if response.double_clicked() || response.middle_clicked() {
            *value = default;
            out.reset = true;
            ui.data_mut(|d| d.remove::<f64>(after_id));
        } else if response.clicked() {
            ui.data_mut(|d| d.insert_temp(after_id, now));
        } else if response.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let delta = response.drag_delta().x / rect.width() * span;
            *value += if fine { delta * 0.1 } else { delta };
        }
        if response.has_focus() {
            let (step, modifiers) = ui.input(|i| {
                let step = f32::from(i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp))
                    - f32::from(i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown));
                (step, i.modifiers)
            });
            if step != 0.0 {
                *value += step * nudge_size(modifiers) * span;
            }
        }
        // A click no second click followed opens the number for typing.
        if let Some(at) = ui.data(|d| d.get_temp::<f64>(after_id)) {
            let waited = now - at;
            if waited >= TYPE_AFTER {
                let text = format!("{:.places$}", clamp_to(*value, min, max));
                ui.data_mut(|d| {
                    d.remove::<f64>(after_id);
                    d.insert_temp(typing_id, text);
                });
                ui.ctx().request_repaint();
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(TYPE_AFTER - waited));
            }
        }
        *value = clamp_to(*value, min, max);
        if (*value - before).abs() > f32::EPSILON || out.reset {
            out.changed = true;
        }
    }

    let font = egui::TextStyle::Body.resolve(ui.style());
    paint_bar(ui, theme, &response, spec, *value, typing.is_none());

    if let Some(mut text) = typing {
        let field = Rect::from_min_max(
            egui::pos2(rect.right() - TYPE_WIDTH - 2.0, rect.top() + 2.0),
            egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
        );
        let edit = ui.put(
            field,
            egui::TextEdit::singleline(&mut text)
                .id(typing_id.with("field"))
                .font(font)
                .align(egui::Align2::RIGHT_CENTER)
                .margin(egui::Margin::symmetric(4, 0))
                .desired_width(field.width()),
        );
        if edit.lost_focus() {
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if !cancelled
                && let Ok(typed) = text.trim().parse::<f32>()
                && typed.is_finite()
                && usable
            {
                *value = clamp_to(typed, min, max);
                out.changed = true;
            }
            ui.data_mut(|d| d.remove::<String>(typing_id));
        } else {
            if !edit.has_focus() {
                edit.request_focus();
            }
            ui.data_mut(|d| d.insert_temp(typing_id, text));
        }
    }
    (response, out)
}

/// Paints a value bar over `response`'s rect, with its number unless a
/// value is being typed over it.
fn paint_bar(
    ui: &Ui,
    theme: &Palette,
    response: &Response,
    spec: &Bar<'_>,
    value: f32,
    show_number: bool,
) {
    let rect = response.rect;
    let (min, max, default) = (spec.min, spec.max, spec.default);
    let span = max - min;
    let usable = span.is_finite() && span.abs() > f32::EPSILON;
    let held = response.dragged() || response.has_focus();
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(BAR_RADIUS);
    let lifted = response.hovered() || response.dragged();
    painter.rect_filled(
        rect,
        radius,
        widgets::hover_fill(ui, response.id, lifted, theme.raised, theme.hover),
    );
    let x_at = |t: f32| rect.left() + rect.width() * t;
    let t = fraction(value, min, max);
    if usable {
        let origin = fill_origin(min, max);
        let (from, to) = (x_at(origin.min(t)), x_at(origin.max(t)));
        if to - from > 0.5 {
            // Rounded where the fill meets the bar's own rounded end.
            let mut corners = CornerRadius::ZERO;
            if origin.min(t) <= 0.0 {
                corners.nw = BAR_RADIUS;
                corners.sw = BAR_RADIUS;
            }
            let fill = if held {
                theme.accent_soft.lerp(theme.accent, 0.15)
            } else {
                theme.accent_soft
            };
            painter.rect_filled(
                Rect::from_x_y_ranges(from..=to, rect.y_range()),
                corners,
                theme::rgb(fill),
            );
        }
        // The smoothed output, while it is still on its way.
        if let Some(output) = spec.output
            && output.is_finite()
            && (output - value).abs() > span.abs() * 0.004
        {
            painter.vline(
                x_at(fraction(output, min, max)),
                rect.y_range(),
                Stroke::new(1.0, theme::rgb(theme.accent).gamma_multiply(0.6)),
            );
        }
        let edge = if held { 1.5 } else { 1.0 };
        painter.rect_filled(
            Rect::from_x_y_ranges(x_at(t) - edge..=x_at(t) + edge, rect.y_range()),
            CornerRadius::ZERO,
            theme::rgb(theme.accent),
        );
        // The default's notch: where reset goes, and that this was changed.
        let home = fraction(default, min, max);
        if (home - t).abs() > 0.01 {
            let x = x_at(home);
            let notch = Stroke::new(1.0, theme::rgb(theme.text_muted));
            painter.vline(x, rect.top()..=rect.top() + 5.0, notch);
            painter.vline(x, rect.bottom() - 5.0..=rect.bottom(), notch);
        }
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, theme::rgb(theme.accent)),
            egui::StrokeKind::Inside,
        );
    }

    let font = egui::TextStyle::Body.resolve(ui.style());
    let center = rect.center().y;
    let mut right = rect.right() - SPACE_SM;
    if show_number {
        let modified = usable && (value - default).abs() > span.abs() * 0.001;
        let places = decimals(min, max);
        let (face, color) = if modified {
            (super::fonts::strong(font.size), theme.text)
        } else {
            (font.clone(), theme.text_muted)
        };
        let galley = painter.layout_no_wrap(
            format!("{value:.places$}{}", spec.unit),
            face,
            theme::rgb(color),
        );
        let size = galley.size();
        painter.galley(
            egui::pos2(right - size.x, center - size.y / 2.0),
            galley,
            theme::rgb(color),
        );
        right -= size.x + SPACE_SM;
    } else {
        right -= TYPE_WIDTH + SPACE_XS;
    }
    if let Some(tag) = spec.tag {
        let galley = painter.layout_no_wrap(
            tag.to_string(),
            super::fonts::strong(9.5),
            theme::rgb(theme.app),
        );
        let size = galley.size();
        let chip = Rect::from_center_size(
            egui::pos2(right - size.x / 2.0 - SPACE_XS, center),
            Vec2::new(size.x + SPACE_SM, size.y + 1.0),
        );
        painter.rect_filled(chip, CornerRadius::same(3), theme::rgb(theme.text_muted));
        painter.galley(chip.center() - size / 2.0, galley, theme::rgb(theme.app));
        right = chip.left() - SPACE_XS;
    }
    let label = widgets::truncated(
        ui,
        spec.label,
        font,
        theme.text,
        right - rect.left() - SPACE_SM,
    );
    let size = label.size();
    painter.galley(
        egui::pos2(rect.left() + SPACE_SM, center - size.y / 2.0),
        label,
        theme::rgb(theme.text),
    );
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
        ui.push_id(index, |ui| match def.kind {
            ControlKind::Slider(mode) | ControlKind::Knob(mode) => {
                let values = (target[0], output[0], mode);
                slider_row(ui, theme, bank, index, &def, values, pins, scale);
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
    }
}

fn describe(response: Response, def: &ControlDef) -> Response {
    match &def.description {
        Some(text) => response.on_hover_text(text),
        None => response,
    }
}

/// A value bar's tooltip: the description, then the range and the default,
/// so the bar itself carries no more words than its name and number.
fn bar_tip(response: Response, def: &ControlDef, spec: &Bar<'_>) -> Response {
    let places = decimals(spec.min, spec.max);
    response.on_hover_ui(|ui| {
        if let Some(text) = &def.description {
            ui.label(text);
        }
        ui.label(format!(
            "{:.places$} to {:.places$}{}, default {:.places$}",
            spec.min, spec.max, spec.unit, spec.default
        ));
    })
}

/// A row's right-click menu: Reset, and pinning as a macro where the row
/// offers it (D210). True when Reset was chosen.
fn row_menu(
    response: &Response,
    def: &ControlDef,
    modified: bool,
    pins: &mut Option<&mut Pins<'_>>,
) -> bool {
    let mut reset = false;
    response.context_menu(|ui| {
        if ui
            .add_enabled(modified, egui::Button::new("Reset to the default"))
            .clicked()
        {
            reset = true;
            ui.close();
        }
        if let Some(pins) = pins.as_deref_mut() {
            let pinned = pins.pinned.contains(&def.name);
            let (label, pin) = if pinned {
                ("Unpin the macro", false)
            } else {
                ("Pin as a macro", true)
            };
            if ui.button(label).clicked() {
                pins.chosen = Some((def.name.clone(), pin));
                ui.close();
            }
        }
    });
    reset
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
    (v, output, mode): (f32, f32, Mode),
    pins: &mut Option<&mut Pins<'_>>,
    scale: Scale,
) {
    let (min, max, default) = (def.min[0], def.max[0], def.default[0]);
    if (max - min).abs() < f32::EPSILON {
        ui.label(format!("{}: {v:.3}", def.name));
        return;
    }
    let mut value = if v.is_finite() { v } else { default };
    // A speed control sets a rate; its output is the phase that rate
    // drives, not a value on this range.
    let (output, unit) = match mode {
        Mode::Plain | Mode::Smooth => (Some(output), ""),
        Mode::Speed | Mode::Traveler => (None, " /s"),
    };
    let name = display_name(&def.name);
    let pinned = pins
        .as_deref()
        .is_some_and(|pins| pins.pinned.contains(&def.name));
    let spec = Bar {
        label: &name,
        min,
        max,
        default,
        output,
        unit,
        tag: pinned.then_some("MACRO"),
        height: scale.bar_height(theme::density()),
    };
    let modified = (value - default).abs() > (max - min).abs() * 0.001;
    let (response, out) = bar(ui, theme, index, &spec, &mut value);
    let response = bar_tip(response, def, &spec);
    if row_menu(&response, def, modified, pins) || out.reset {
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
    // A chip the bars' height (D510): the whole row clicks, a lit dot and
    // ON say it is on, and the row takes the accent's soft fill.
    let on = def.normalize(0, v) >= 0.5;
    let name = display_name(&def.name);
    let size = Vec2::new(
        ui.available_width().max(48.0),
        scale.bar_height(theme::density()),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, name.as_str()));
    let t = super::motion::toggle(ui.ctx(), response.id, on, super::motion::INSTANT);
    let rest = theme.raised.lerp(theme.accent_soft, t);
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(BAR_RADIUS);
    painter.rect_filled(
        rect,
        radius,
        widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            rest,
            rest.lerp(theme.hover, 0.6),
        ),
    );
    let ring = if response.has_focus() {
        Stroke::new(1.5, theme::rgb(theme.accent))
    } else {
        Stroke::new(1.0, theme::rgb(theme.raised.lerp(theme.accent, 0.45 * t)))
    };
    painter.rect_stroke(rect, radius, ring, egui::StrokeKind::Inside);
    let center = rect.center().y;
    let dot = egui::pos2(rect.left() + SPACE_SM + 3.5, center);
    painter.circle_filled(dot, 3.5, theme::rgb(theme.control.lerp(theme.accent, t)));
    let small = egui::TextStyle::Small.resolve(ui.style());
    let state = painter.layout_no_wrap(
        if on { "ON" } else { "OFF" }.to_string(),
        small,
        theme::rgb(if on { theme.accent } else { theme.text_faint }),
    );
    let state_size = state.size();
    let state_at = egui::pos2(
        rect.right() - SPACE_SM - state_size.x,
        center - state_size.y / 2.0,
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let left = dot.x + 3.5 + SPACE_SM;
    let label = widgets::truncated(ui, &name, font, theme.text, state_at.x - left - SPACE_SM);
    let label_size = label.size();
    painter.galley(
        egui::pos2(left, center - label_size.y / 2.0),
        label,
        theme::rgb(theme.text),
    );
    painter.galley(state_at, state, theme::rgb(theme.text_muted));
    let response = describe(response, def);
    let at_default = (def.normalize(0, def.default[0]) >= 0.5) == on;
    if row_menu(&response, def, !at_default, pins) {
        bank.set_default(index);
    } else if response.clicked() {
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
        // A bang is a button to hit, so its name is in capitals (owner's call).
        let label = display_name(&def.name).to_uppercase();
        let response = bang_button_sized(ui, theme, &label, scale.bang_height());
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
    // A row the bars' height (D510): the name, the hex value, and the
    // swatch at the value's place, which opens the picker.
    let size = Vec2::new(
        ui.available_width().max(48.0),
        scale.bar_height(theme::density()),
    );
    let (rect, row) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(BAR_RADIUS),
        widgets::hover_fill(ui, row.id, row.hovered(), theme.raised, theme.hover),
    );
    let swatch_size = Vec2::new((size.y * 1.4).round(), size.y - 8.0);
    let swatch_rect = Rect::from_min_size(
        egui::pos2(
            rect.right() - SPACE_XS - swatch_size.x,
            rect.center().y - swatch_size.y / 2.0,
        ),
        swatch_size,
    );
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let hex = painter.layout_no_wrap(
        format!(
            "#{:02X}{:02X}{:02X}",
            channel(v[0]),
            channel(v[1]),
            channel(v[2])
        ),
        egui::TextStyle::Small.resolve(ui.style()),
        theme::rgb(theme.text_muted),
    );
    let hex_size = hex.size();
    let hex_at = egui::pos2(
        swatch_rect.left() - SPACE_SM - hex_size.x,
        rect.center().y - hex_size.y / 2.0,
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name = display_name(&def.name);
    let label = widgets::truncated(
        ui,
        &name,
        font,
        theme.text,
        hex_at.x - rect.left() - SPACE_SM * 2.0,
    );
    let label_size = label.size();
    painter.galley(
        egui::pos2(rect.left() + SPACE_SM, rect.center().y - label_size.y / 2.0),
        label,
        theme::rgb(theme.text),
    );
    painter.galley(hex_at, hex, theme::rgb(theme.text_muted));
    let mut rgb = v;
    let picked = ui
        .scope_builder(egui::UiBuilder::new().max_rect(swatch_rect), |ui| {
            swatch(ui, theme, &mut rgb, swatch_size)
        })
        .inner;
    if describe(picked, def).changed() {
        bank.set(index, None, &rgb);
    }
    let modified = (0..3).any(|c| def.default[c] != v[c]);
    if row_menu(&describe(row, def), def, modified, pins) {
        bank.set_default(index);
    }
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

    /// Runs one egui pass at `time` with `events` and reports what a value
    /// bar over 0..10 (default `default`) did.
    fn run_bar(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        value: &mut f32,
        default: f32,
    ) -> (Rect, RailOut) {
        let theme = crate::palette::fallback().clone();
        let mut out = RailOut::default();
        let mut rect = Rect::NOTHING;
        let mut raw = crate::ui::testing::input(Vec2::new(400.0, 200.0), events);
        raw.time = Some(time);
        let _ = crate::ui::testing::run(ctx, raw, |ui| {
            let spec = Bar {
                label: "Swirl",
                min: 0.0,
                max: 10.0,
                default,
                output: None,
                unit: "",
                tag: None,
                height: 24.0,
            };
            let (response, result) = bar(ui, &theme, 0_u8, &spec, value);
            rect = response.rect;
            out = result;
        });
        (rect, out)
    }

    /// One press and release at `at`, starting at `time`; returns the time
    /// after.
    fn click_bar(ctx: &egui::Context, time: f64, at: egui::Pos2, value: &mut f32) -> f64 {
        let _ = run_bar(
            ctx,
            time,
            vec![egui::Event::PointerMoved(at), press(at, true)],
            value,
            5.0,
        );
        let _ = run_bar(ctx, time + 0.05, vec![press(at, false)], value, 5.0);
        time + 0.05
    }

    #[test]
    fn a_bar_drag_is_relative_so_taking_hold_does_not_jump() {
        let ctx = egui::Context::default();
        let mut value = 2.0;
        let (rect, _) = run_bar(&ctx, 0.0, Vec::new(), &mut value, 5.0);
        // Take hold far right of the value, then move left a tenth of the
        // bar: the value moves by up to a tenth of the range, from where it
        // was (the drag threshold keeps a little).
        let start = egui::pos2(rect.left() + rect.width() * 0.9, rect.center().y);
        let mut time = 0.0;
        let mut at = start;
        let _ = run_bar(
            &ctx,
            time,
            vec![egui::Event::PointerMoved(at), press(at, true)],
            &mut value,
            5.0,
        );
        for _ in 0..10 {
            time += 0.016;
            at.x -= rect.width() * 0.01;
            let _ = run_bar(
                &ctx,
                time,
                vec![egui::Event::PointerMoved(at)],
                &mut value,
                5.0,
            );
        }
        let _ = run_bar(&ctx, time + 0.016, vec![press(at, false)], &mut value, 5.0);
        assert!(
            value < 2.0 && value > 0.9,
            "a drag left from 2.0 went down without jumping to the pointer: {value}"
        );
    }

    #[test]
    fn a_bar_click_opens_the_number_and_enter_sets_it() {
        let ctx = egui::Context::default();
        let mut value = 2.0;
        let (rect, _) = run_bar(&ctx, 0.0, Vec::new(), &mut value, 5.0);
        let at = rect.center();
        let time = click_bar(&ctx, 0.0, at, &mut value);
        assert!(
            (value - 2.0).abs() < f32::EPSILON,
            "a click does not move the value: {value}"
        );
        // After the double-click window the field opens, then takes focus.
        let _ = run_bar(&ctx, time + 0.4, Vec::new(), &mut value, 5.0);
        let _ = run_bar(&ctx, time + 0.45, Vec::new(), &mut value, 5.0);
        let select_all = egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        let _ = run_bar(
            &ctx,
            time + 0.5,
            vec![select_all, egui::Event::Text("7.25".to_string())],
            &mut value,
            5.0,
        );
        let enter = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let (_, out) = run_bar(&ctx, time + 0.55, vec![enter], &mut value, 5.0);
        assert!(out.changed, "Enter reports the change");
        assert!((value - 7.25).abs() < 1e-4, "typed 7.25, got {value}");
    }

    #[test]
    fn a_bar_double_click_resets_and_opens_no_field() {
        let ctx = egui::Context::default();
        let mut value = 9.0;
        let (rect, _) = run_bar(&ctx, 0.0, Vec::new(), &mut value, 5.0);
        let at = rect.center();
        let time = click_bar(&ctx, 0.0, at, &mut value);
        let _ = click_bar(&ctx, time + 0.05, at, &mut value);
        assert!(
            (value - 5.0).abs() < f32::EPSILON,
            "reset to the default: {value}"
        );
        // Long after, a typed character changes nothing: no field opened.
        let _ = run_bar(&ctx, 2.0, Vec::new(), &mut value, 5.0);
        let _ = run_bar(
            &ctx,
            2.1,
            vec![egui::Event::Text("1".to_string())],
            &mut value,
            5.0,
        );
        assert!(
            (value - 5.0).abs() < f32::EPSILON,
            "nothing typed into a field: {value}"
        );
    }

    #[test]
    fn a_bipolar_bar_fills_from_zero_and_others_from_the_left() {
        assert!(
            (fill_origin(-1.0, 1.0) - 0.5).abs() < 1e-6,
            "symmetric range"
        );
        assert!(
            (fill_origin(-1.0, 3.0) - 0.25).abs() < 1e-6,
            "lopsided range"
        );
        assert_eq!(fill_origin(0.0, 1.0), 0.0, "starts at zero");
        assert_eq!(fill_origin(0.4, 1.6), 0.0, "all positive");
        assert_eq!(fill_origin(-2.0, -1.0), 0.0, "all negative");
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
