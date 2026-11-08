// ============================================================================
// Module:       strobe_app::ui::controls
// Description:  One widget per scene control kind, written against the
//               control bank: the value bar, the toggle switch, bang, XY
//               pad, colour swatch and dropdown, the macro cards' slider rail, and
//               the grouping of controls by UI_GROUP.
//
// Dependencies: egui; strobe-scene (ControlBank, ControlDef, ControlKind,
//               Mode); super::{theme, widgets, fonts, motion}
// ============================================================================

//! The control widgets (UI_DESIGN section 10).
//!
//! One widget per [`ControlKind`], chosen by an exhaustive match so a new
//! control type cannot silently render nothing:
//!
//! | Control | Widget | Writes |
//! |---|---|---|
//! | slider, knob | a [`bar`] over the control's real range | `set` |
//! | toggle | a property row with a switch at its right end | `set` to `MIN` or `MAX` |
//! | bang, bang counter | a row-sized outlined button, its name in capitals | `press` |
//! | xy | a property row with the readout, then a pad the rows' width | `set` of both components |
//! | color | a property row with the hex value and a swatch that opens a picker | `set` of three components |
//! | dropdown | a property row with its options side by side at its right end, or a combo box past five or when they do not fit (D563) | `set` to the option index |
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
//! ## One row grammar (D510)
//!
//! Every control is a row in one frame: the bar's height, its raised
//! surface and rounded ends, its name at the same left inset, and its value
//! or widget ending at the same right inset. A slider is a bar; anything
//! that is not a point on a range is a *property row* ([`property_row`]):
//! the same frame with no fill, its switch, swatch or list at the right end
//! where a bar's number sits. A bang is the one row that is a button: the
//! frame outlined and its name in capitals in the middle. So a group's rows
//! start and end on the same two lines whatever their kinds.
//!
//! A control a MIDI or OSC mapping drives says so on its row (D563): the
//! source's short name (`CC 21`, `/1/fader1`) in an outlined mark, left of
//! the number or the widget, and every mapping in full in the tooltip. It is
//! outlined where `MACRO` is filled, so the two read apart.
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

use super::theme::{self, Density, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::palette::Palette;
use egui::{CornerRadius, Key, Modifiers, Rect, Response, Sense, Stroke, Ui, Vec2};
use strobe_scene::control::{Bang, ControlBank, ControlDef, ControlKind, Mode};

/// The xy pad's height in the side panel, in points.
const PAD_HEIGHT: f32 = 108.0;

/// The height of a slider rail's hit area.
const RAIL_HEIGHT: f32 = 18.0;

/// A value bar's corner radius: a step under the buttons', so
/// a column of rows reads as rows.
const BAR_RADIUS: u8 = 4;

/// How long a click on a value bar waits before it opens the number for
/// typing: egui's double-click window, so the first click of a double
/// click (reset) never opens the field.
const TYPE_AFTER: f64 = 0.3;

/// The width of the field a value is typed into.
const TYPE_WIDTH: f32 = 72.0;

/// The most options a dropdown shows side by side (D563); past this it is a
/// combo box.
const SEGMENTS_MAX: usize = 5;

/// The widest a mapping's mark grows before its text is cut short: an OSC
/// address can be long, and the name needs the room more.
const MARK_MAX: f32 = 96.0;

/// The least room a row keeps for its name when a dropdown decides whether
/// its options fit side by side.
const NAME_ROOM: f32 = 72.0;

/// The narrowest an option side by side is drawn, so a one-letter label is
/// still a target.
const SEGMENT_MIN: f32 = 28.0;

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
    /// A value bar's height at `density` (D510):
    /// the window's density setting packs them as it packs every list.
    pub const fn bar_height(self, density: Density) -> f32 {
        match (self, density) {
            (Self::Panel, Density::Comfortable) => 24.0,
            (Self::Panel, Density::Compact) => 20.0,
            (Self::Touch, Density::Comfortable) => 30.0,
            (Self::Touch, Density::Compact) => 24.0,
        }
    }

    /// The xy pad: the row's full width, so its edges are the rows' edges
    /// (D510), and a height that suits it.
    fn pad_size(self, width: f32) -> Vec2 {
        match self {
            Self::Panel => Vec2::new(width, PAD_HEIGHT),
            Self::Touch => Vec2::new(width, (width * 0.5).clamp(PAD_HEIGHT, 200.0)),
        }
    }

    /// A bang button's height at `density`.
    pub const fn button_height(self, density: Density) -> f32 {
        match (self, density) {
            (Self::Panel, Density::Comfortable) => 26.0,
            (Self::Panel, Density::Compact) => 22.0,
            (Self::Touch, Density::Comfortable) => 36.0,
            (Self::Touch, Density::Compact) => 30.0,
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
    /// The mapping that drives it, as its mark shows it (D563).
    pub mark: Option<&'a str>,
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
    if let Some(mark) = spec.mark {
        right = paint_mark(
            ui,
            &painter,
            theme,
            mark,
            (right, center),
            right - rect.left() - NAME_ROOM,
        );
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

/// A mapping that drives a control, as its row shows it (D563).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// On the row: `CC 21`, an OSC address.
    pub tag: String,
    /// In the tooltip: the source with its kind, channel and device.
    pub full: String,
}

/// What a row's mark says for `marks`: the first mapping's tag, and how many
/// more there are. `None` for a control nothing drives.
pub fn mark_text(marks: &[Mark]) -> Option<String> {
    let first = marks.first()?;
    Some(match marks.len() {
        1 => first.tag.clone(),
        n => format!("{} +{}", first.tag, n - 1),
    })
}

/// A mark's font: the small text, regular, so it reads under the name.
fn mark_font(ui: &Ui) -> egui::FontId {
    egui::TextStyle::Small.resolve(ui.style())
}

/// How wide `text`'s mark is drawn, at most [`MARK_MAX`].
fn mark_width(ui: &Ui, theme: &Palette, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        mark_font(ui),
        theme::rgb(theme.text_muted),
    );
    (galley.size().x + SPACE_SM).min(MARK_MAX)
}

/// Paints a mapping's mark with its right edge and centre at `at`, cut short
/// to fit `room` (and [`MARK_MAX`]), and returns where the next thing to its
/// left ends. With too little room it paints nothing and returns `at`'s x.
fn paint_mark(
    ui: &Ui,
    painter: &egui::Painter,
    theme: &Palette,
    text: &str,
    at: (f32, f32),
    room: f32,
) -> f32 {
    let (right, center) = at;
    let room = room.min(MARK_MAX);
    if room < 28.0 {
        return right;
    }
    let galley = widgets::truncated(ui, text, mark_font(ui), theme.text_muted, room - SPACE_SM);
    let size = galley.size();
    let chip = Rect::from_min_max(
        egui::pos2(right - size.x - SPACE_SM, center - size.y / 2.0 - 1.0),
        egui::pos2(right, center + size.y / 2.0 + 1.0),
    );
    painter.rect_stroke(
        chip,
        CornerRadius::same(3),
        Stroke::new(1.0, theme::rgb(theme.border_strong)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        chip.center() - size / 2.0,
        galley,
        theme::rgb(theme.text_muted),
    );
    chip.left() - SPACE_XS
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

/// What a bank's rows offer besides their controls: pinning a control as a
/// Performance Macro from its row (D210), which the context menu offers,
/// and the marks of the mappings that drive each control (D563).
pub struct Extras<'a> {
    /// The bank's controls that already have a pinned macro, by name.
    pub pinned: &'a [String],
    /// What was chosen this frame: a control's name, and pin (`true`) or
    /// unpin.
    pub chosen: Option<(String, bool)>,
    /// The mappings that drive each control, by the control's index in the
    /// bank. Shorter than the bank, or empty, where nothing is mapped.
    pub marks: &'a [Vec<Mark>],
}

/// The marks of the control at `index`, copied out so the row can still
/// take the extras mutably for its menu: none without extras.
fn marks_of(pins: &Option<&mut Extras<'_>>, index: usize) -> Vec<Mark> {
    pins.as_deref()
        .and_then(|extras| extras.marks.get(index))
        .cloned()
        .unwrap_or_default()
}

/// Gives `response` (a control's name or button) the pin menu.
fn pin_menu(response: &Response, def: &ControlDef, pins: &mut Option<&mut Extras<'_>>) {
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

/// Draws every control in `members` (indices into the bank). Group headers
/// are the inspector's business; this draws the rows, at `scale`. With
/// `pins`, a control's name offers to pin it as a macro.
pub fn rows(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    members: &[usize],
    mut pins: Option<&mut Extras<'_>>,
    scale: Scale,
) {
    let pins = &mut pins;
    for &index in members {
        let Some(rect) = control(ui, theme, bank, index, pins, scale) else {
            continue;
        };
        // A controlling Strobe draws its own copy of the row here (D515).
        crate::remote_controls::noted(ui, bank, index, rect, matches!(scale, Scale::Touch));
    }
}

/// Draws the control at `index` with its kind's widget, and returns the
/// rect it took, or `None` when the bank has no control there.
fn control(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) -> Option<Rect> {
    let def = bank.defs().get(index).cloned()?;
    let target = bank.target(index).copied().unwrap_or_default();
    let output = bank.output(index).copied().unwrap_or_default();
    let drawn = ui.push_id(index, |ui| match def.kind {
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
            dropdown_row(ui, theme, bank, index, &def, target[0], pins, scale);
        }
    });
    Some(drawn.response.rect)
}

fn describe(response: Response, def: &ControlDef) -> Response {
    match &def.description {
        Some(text) => response.on_hover_text(text),
        None => response,
    }
}

/// Every mapping that drives a control, one line each, for its tooltip.
fn mapped_lines(ui: &mut Ui, marks: &[Mark]) {
    for mark in marks {
        ui.label(format!("Mapped to {}", mark.full));
    }
}

/// A row's tooltip: the description and every mapping that drives the
/// control (D563). No tooltip when there is neither.
fn row_tip(response: Response, def: &ControlDef, marks: &[Mark]) -> Response {
    if def.description.is_none() && marks.is_empty() {
        return response;
    }
    response.on_hover_ui(|ui| {
        if let Some(text) = &def.description {
            ui.label(text);
        }
        mapped_lines(ui, marks);
    })
}

/// A value bar's tooltip: the description, then the range and the default,
/// then the mappings, so the bar itself carries no more words than its name
/// and number.
fn bar_tip(response: Response, def: &ControlDef, spec: &Bar<'_>, marks: &[Mark]) -> Response {
    let places = decimals(spec.min, spec.max);
    response.on_hover_ui(|ui| {
        if let Some(text) = &def.description {
            ui.label(text);
        }
        ui.label(format!(
            "{:.places$} to {:.places$}{}, default {:.places$}",
            spec.min, spec.max, spec.unit, spec.default
        ));
        mapped_lines(ui, marks);
    })
}

/// A row's right-click menu: Reset, and pinning as a macro where the row
/// offers it (D210). True when Reset was chosen.
fn row_menu(
    response: &Response,
    def: &ControlDef,
    modified: bool,
    pins: &mut Option<&mut Extras<'_>>,
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
    pins: &mut Option<&mut Extras<'_>>,
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
    let marks = marks_of(pins, index);
    let mark = mark_text(&marks);
    let spec = Bar {
        label: &name,
        min,
        max,
        default,
        output,
        unit,
        tag: pinned.then_some("MACRO"),
        mark: mark.as_deref(),
        height: scale.bar_height(theme::density()),
    };
    let modified = (value - default).abs() > (max - min).abs() * 0.001;
    let (response, out) = bar(ui, theme, index, &spec, &mut value);
    let response = bar_tip(response, def, &spec, &marks);
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
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) {
    // A property row with the switch at its right end (D510); a click
    // anywhere on the row flips it.
    let on = def.normalize(0, v) >= 0.5;
    let name = display_name(&def.name);
    let switch = switch_size(scale.bar_height(theme::density()));
    let marks = marks_of(pins, index);
    let response = property_row(
        ui,
        theme,
        &name,
        scale,
        switch.x,
        mark_text(&marks).as_deref(),
    );
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, name.as_str()));
    let rect = right_slot(response.rect, switch);
    // The switch's slide has an id of its own: the row's hover fill is
    // keyed by `response.id`, and one animation under both drew the switch
    // on whenever the pointer was over the row.
    let t = super::motion::toggle(
        ui.ctx(),
        response.id.with("switch"),
        on,
        super::motion::INSTANT,
    );
    let half = switch.y / 2.0;
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(half as u8),
        theme::rgb(theme.control.lerp(theme.accent, t)),
    );
    painter.circle_filled(
        egui::pos2(
            rect.left() + half + t * (rect.width() - switch.y),
            rect.center().y,
        ),
        half - 2.0,
        theme::rgb(theme.text.lerp(theme.text_on_accent, t)),
    );
    let response = row_tip(response, def, &marks);
    let at_default = (def.normalize(0, def.default[0]) >= 0.5) == on;
    if row_menu(&response, def, !at_default, pins) {
        bank.set_default(index);
    } else if response.clicked() {
        let next = if on { def.min[0] } else { def.max[0] };
        bank.set(index, Some(0), &[next]);
    }
}

/// A toggle's switch, sized to sit inside a row `height` tall.
fn switch_size(height: f32) -> Vec2 {
    let tall = (height - 10.0).max(12.0);
    Vec2::new((tall * 1.8).round(), tall)
}

/// A row in a value bar's frame for a control that is not a point on a
/// range (D510): the bar's height, raised surface and rounded ends, and
/// the name at the bar's left inset. The caller draws its widget,
/// `widget` wide, at the right end ([`right_slot`]), where a bar's number
/// sits, so every row of a group starts and ends on the same two lines. A
/// mapping's `mark` (D563) sits just left of the widget.
fn property_row(
    ui: &mut Ui,
    theme: &Palette,
    name: &str,
    scale: Scale,
    widget: f32,
    mark: Option<&str>,
) -> Response {
    let height = scale.bar_height(theme::density());
    let size = Vec2::new(ui.available_width().max(48.0), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(BAR_RADIUS);
    painter.rect_filled(
        rect,
        radius,
        widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            theme.raised,
            theme.hover,
        ),
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, theme::rgb(theme.accent)),
            egui::StrokeKind::Inside,
        );
    }
    let mut right = rect.right() - SPACE_SM - widget - SPACE_SM;
    if let Some(mark) = mark {
        right = paint_mark(
            ui,
            &painter,
            theme,
            mark,
            (right, rect.center().y),
            right - rect.left() - NAME_ROOM,
        );
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label = widgets::truncated(ui, name, font, theme.text, right - rect.left() - SPACE_SM);
    let label_size = label.size();
    painter.galley(
        egui::pos2(rect.left() + SPACE_SM, rect.center().y - label_size.y / 2.0),
        label,
        theme::rgb(theme.text),
    );
    response
}

/// Where a widget `size` large sits at a row's right end: inset as a bar's
/// number is, centred on the row.
fn right_slot(row: Rect, size: Vec2) -> Rect {
    Rect::from_min_size(
        egui::pos2(
            row.right() - SPACE_SM - size.x,
            row.center().y - size.y / 2.0,
        ),
        size,
    )
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
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) {
    // A button the row's width and height (D510): outlined and labelled in
    // capitals in its middle, where every other row has its name at the
    // left, so it reads as a thing to hit. A counter shows its hits at the
    // right end, where a bar's number sits.
    let size = Vec2::new(
        ui.available_width().max(48.0),
        scale.bar_height(theme::density()),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let pressed = response.is_pointer_button_down_on();
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(BAR_RADIUS);
    let fill = if pressed {
        theme::rgb(theme.accent)
    } else {
        widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            theme.raised,
            theme.hover,
        )
    };
    painter.rect_filled(rect, radius, fill);
    let edge = if pressed || response.has_focus() {
        theme.accent
    } else {
        theme.border_strong
    };
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::rgb(edge)),
        egui::StrokeKind::Inside,
    );
    let ink = if pressed {
        theme.text_on_accent
    } else {
        theme.text
    };
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label = painter.layout_no_wrap(
        display_name(&def.name).to_uppercase(),
        super::fonts::strong(font.size),
        theme::rgb(ink),
    );
    let label_right = rect.center().x + label.size().x / 2.0;
    painter.galley(rect.center() - label.size() / 2.0, label, theme::rgb(ink));
    let mut right = rect.right() - SPACE_SM;
    match kind {
        Bang::Counter | Bang::CounterSmooth => {
            let count = painter.layout_no_wrap(
                format!("{output:.0}"),
                font,
                theme::rgb(if pressed { ink } else { theme.text_muted }),
            );
            let count_size = count.size();
            painter.galley(
                egui::pos2(right - count_size.x, rect.center().y - count_size.y / 2.0),
                count,
                theme::rgb(theme.text_muted),
            );
            right -= count_size.x + SPACE_SM;
        }
        Bang::Plain | Bang::Smooth => {}
    }
    // The mark at the right end, clear of the name in the middle.
    let marks = marks_of(pins, index);
    if let Some(mark) = mark_text(&marks) {
        // Where the mark ends is not needed: nothing is drawn left of it.
        let _ = paint_mark(
            ui,
            &painter,
            theme,
            &mark,
            (right, rect.center().y),
            right - label_right - SPACE_SM,
        );
    }
    pin_menu(&response, def, pins);
    if row_tip(response, def, &marks).clicked() {
        bank.press(index);
    }
}

/// A momentary button with a visible pressed state.
pub fn bang_button(ui: &mut Ui, theme: &Palette, name: &str) -> Response {
    bang_button_sized(
        ui,
        theme,
        name,
        Scale::Panel.button_height(theme::density()),
    )
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
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) {
    // A property row with the readout where a bar's number sits, then the
    // pad under it, the rows' full width (D510).
    const READOUT: f32 = 96.0;
    let modified = def.default[0] != v[0] || def.default[1] != v[1];
    let name = display_name(&def.name);
    let marks = marks_of(pins, index);
    let row = property_row(
        ui,
        theme,
        &name,
        scale,
        READOUT,
        mark_text(&marks).as_deref(),
    );
    ui.painter().text(
        egui::pos2(row.rect.right() - SPACE_SM, row.rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.2}, {:.2}", v[0], v[1]),
        egui::TextStyle::Body.resolve(ui.style()),
        theme::rgb(if modified {
            theme.text
        } else {
            theme.text_muted
        }),
    );
    if row_menu(&row_tip(row, def, &marks), def, modified, pins) {
        bank.set_default(index);
    }
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
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) {
    // A property row with the swatch at its right end and the hex value
    // before it (D510). The swatch opens the picker.
    const HEX_ROOM: f32 = 64.0;
    let height = scale.bar_height(theme::density());
    let swatch_size = Vec2::new((height * 1.6).round(), height - 8.0);
    let name = display_name(&def.name);
    let marks = marks_of(pins, index);
    let row = property_row(
        ui,
        theme,
        &name,
        scale,
        swatch_size.x + HEX_ROOM,
        mark_text(&marks).as_deref(),
    );
    let slot = right_slot(row.rect, swatch_size);
    let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().text(
        egui::pos2(slot.left() - SPACE_SM, row.rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!(
            "#{:02X}{:02X}{:02X}",
            channel(v[0]),
            channel(v[1]),
            channel(v[2])
        ),
        egui::TextStyle::Small.resolve(ui.style()),
        theme::rgb(theme.text_muted),
    );
    let mut rgb = v;
    let picked = ui.put(slot, |ui: &mut Ui| swatch(ui, theme, &mut rgb, swatch_size));
    if describe(picked, def).changed() {
        bank.set(index, None, &rgb);
    }
    let modified = (0..3).any(|c| def.default[c] != v[c]);
    if row_menu(&row_tip(row, def, &marks), def, modified, pins) {
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

#[expect(
    clippy::too_many_arguments,
    reason = "a row's whole context, as its siblings take"
)]
fn dropdown_row(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    index: usize,
    def: &ControlDef,
    v: f32,
    pins: &mut Option<&mut Extras<'_>>,
    scale: Scale,
) {
    // A property row with the options side by side at its right end when
    // there are few enough and they fit (D563), or a combo box (D510).
    let selected = (v.round().max(0.0) as usize).min(def.labels.len().saturating_sub(1));
    let height = scale.bar_height(theme::density());
    let name = display_name(&def.name);
    let marks = marks_of(pins, index);
    let mark = mark_text(&marks);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let natural: Vec<f32> = def
        .labels
        .iter()
        .map(|label| {
            let galley =
                ui.painter()
                    .layout_no_wrap(label.clone(), font.clone(), theme::rgb(theme.text));
            (galley.size().x + SPACE_SM * 2.0).max(SEGMENT_MIN)
        })
        .collect();
    let marked = mark
        .as_deref()
        .map_or(0.0, |mark| mark_width(ui, theme, mark) + SPACE_XS);
    let room = ui.available_width().max(48.0) - SPACE_SM * 3.0 - NAME_ROOM - marked;
    let row = match segment_widths(&natural, room) {
        Some(cells) => {
            let total: f32 = cells.iter().sum();
            let row = property_row(ui, theme, &name, scale, total, mark.as_deref());
            let slot = right_slot(row.rect, Vec2::new(total, height - 2.0));
            if let Some(option) = segments(ui, theme, slot, &def.labels, &cells, selected) {
                bank.set(index, Some(0), &[option as f32]);
            }
            row
        }
        None => {
            let current = def.labels.get(selected).cloned().unwrap_or_default();
            let width = (ui.available_width() * 0.5).clamp(80.0, 200.0);
            let row = property_row(ui, theme, &name, scale, width, mark.as_deref());
            let slot = right_slot(row.rect, Vec2::new(width, height - 4.0));
            let picked = ui.put(slot, |ui: &mut Ui| {
                // No taller than the row, at the compact density too.
                ui.spacing_mut().interact_size.y = slot.height();
                egui::ComboBox::from_id_salt(("dropdown", index))
                    .width(width)
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (option, label) in def.labels.iter().enumerate() {
                            if ui.selectable_label(option == selected, label).clicked() {
                                bank.set(index, Some(0), &[option as f32]);
                            }
                        }
                    })
                    .response
            });
            describe(picked, def);
            row
        }
    };
    let modified = (def.default[0] - v).abs() > f32::EPSILON;
    if row_menu(&row_tip(row, def, &marks), def, modified, pins) {
        bank.set_default(index);
    }
}

/// The widths of a dropdown's options side by side (D563), from each
/// label's natural width, in `room`: equal cells when the widest fits that
/// way, each its own width when only that fits, and `None` (a combo box)
/// for no options, more than [`SEGMENTS_MAX`], or options that do not fit.
pub fn segment_widths(natural: &[f32], room: f32) -> Option<Vec<f32>> {
    if natural.is_empty() || natural.len() > SEGMENTS_MAX || !room.is_finite() {
        return None;
    }
    let widest = natural.iter().copied().fold(0.0_f32, f32::max);
    if widest * natural.len() as f32 <= room {
        return Some(vec![widest; natural.len()]);
    }
    let sum: f32 = natural.iter().sum();
    (sum <= room).then(|| natural.to_vec())
}

/// A dropdown's options side by side in `slot`, `cells` wide each, the
/// chosen one filled in the accent (D563). Each option is a target of its
/// own: a click, or Space or Enter with it focused, chooses it. Returns the
/// option chosen this frame.
fn segments(
    ui: &mut Ui,
    theme: &Palette,
    slot: Rect,
    labels: &[String],
    cells: &[f32],
    selected: usize,
) -> Option<usize> {
    let painter = ui.painter().clone();
    let radius = CornerRadius::same(BAR_RADIUS - 1);
    painter.rect_filled(slot, radius, theme::rgb(theme.app));
    painter.rect_stroke(
        slot,
        radius,
        Stroke::new(1.0, theme::rgb(theme.border)),
        egui::StrokeKind::Inside,
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut left = slot.left();
    let mut chosen = None;
    for (option, (label, width)) in labels.iter().zip(cells).enumerate() {
        let cell = Rect::from_min_size(
            egui::pos2(left, slot.top()),
            Vec2::new(*width, slot.height()),
        )
        .shrink2(Vec2::new(2.0, 1.5));
        left += width;
        let response = ui.interact(
            cell,
            ui.scope_id().with(("segment", option)),
            Sense::click(),
        );
        let on = option == selected;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label.as_str())
        });
        let fill = if on {
            theme::rgb(theme.accent)
        } else {
            widgets::hover_fill(ui, response.id, response.hovered(), theme.app, theme.hover)
        };
        painter.rect_filled(cell, CornerRadius::same(BAR_RADIUS - 2), fill);
        if response.has_focus() {
            painter.rect_stroke(
                cell,
                CornerRadius::same(BAR_RADIUS - 2),
                Stroke::new(1.5, theme::rgb(theme.accent_hover)),
                egui::StrokeKind::Inside,
            );
        }
        let ink = if on {
            theme.text_on_accent
        } else {
            theme.text_muted
        };
        let text = widgets::truncated(ui, label, font.clone(), ink, cell.width() - SPACE_XS);
        painter.galley(cell.center() - text.size() / 2.0, text, theme::rgb(ink));
        if response.clicked() && !on {
            chosen = Some(option);
        }
    }
    chosen
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
                mark: None,
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

    /// Every rectangle fill in `shapes`, nested lists opened.
    fn fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Color32> {
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
            match shape {
                egui::Shape::Rect(rect) => out.push(rect.fill),
                egui::Shape::Vec(inner) => {
                    for shape in inner {
                        walk(shape, out);
                    }
                }
                egui::Shape::Noop
                | egui::Shape::Circle(_)
                | egui::Shape::Ellipse(_)
                | egui::Shape::LineSegment { .. }
                | egui::Shape::Path(_)
                | egui::Shape::Band(_)
                | egui::Shape::Text(_)
                | egui::Shape::Mesh(_)
                | egui::Shape::QuadraticBezier(_)
                | egui::Shape::CubicBezier(_)
                | egui::Shape::Callback(_) => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn hovering_an_off_toggle_neither_flips_it_nor_draws_it_on() -> Result<()> {
        let json = r#"{ "CONTROLS": [ { "NAME": "invert", "TYPE": "toggle" } ] }"#;
        let scene = SceneDef::parse(json, "t")?;
        let mut bank = ControlBank::new(scene.controls)?;
        let theme = crate::palette::fallback().clone();
        let ctx = egui::Context::default();
        // Over the row, long enough for any hover animation to finish.
        let over = egui::pos2(100.0, 12.0);
        let mut last = Vec::new();
        for frame in 0..12 {
            let mut raw = crate::ui::testing::input(
                Vec2::new(400.0, 200.0),
                vec![egui::Event::PointerMoved(over)],
            );
            raw.time = Some(f64::from(frame) * 0.05);
            let pass = crate::ui::testing::run(&ctx, raw, |ui| {
                rows(ui, &theme, &mut bank, &[0], None, Scale::Touch);
            });
            last = pass.shapes;
        }
        let value = bank.target(0).copied().unwrap_or_default()[0];
        assert!(
            value.abs() < f32::EPSILON,
            "hover flipped the toggle to {value}"
        );
        let drawn = fills(&last);
        assert!(
            !drawn.contains(&theme::rgb(theme.accent)),
            "a hovered off toggle was drawn with the accent (on): {drawn:?}"
        );
        assert!(
            drawn.contains(&theme::rgb(theme.control)),
            "the off switch's track is drawn in the control colour: {drawn:?}"
        );
        Ok(())
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

    #[test]
    fn short_options_sit_side_by_side_and_long_or_many_do_not() {
        assert_eq!(
            segment_widths(&[30.0, 50.0, 40.0], 200.0),
            Some(vec![50.0; 3]),
            "equal cells when the widest fits three times"
        );
        assert_eq!(
            segment_widths(&[30.0, 90.0, 40.0], 200.0),
            Some(vec![30.0, 90.0, 40.0]),
            "each its own width when only that fits"
        );
        assert_eq!(
            segment_widths(&[80.0, 90.0, 70.0], 200.0),
            None,
            "a combo box when they do not fit"
        );
        assert_eq!(
            segment_widths(&[30.0; 6], 1000.0),
            None,
            "a combo box past five options"
        );
        assert_eq!(segment_widths(&[], 1000.0), None, "no options");
        assert_eq!(
            segment_widths(&[30.0], f32::NAN),
            None,
            "no room to speak of"
        );
    }

    #[test]
    fn a_mark_names_the_first_mapping_and_counts_the_rest() {
        let mark = |tag: &str| Mark {
            tag: tag.to_string(),
            full: format!("MIDI {tag} ch 1"),
        };
        assert_eq!(mark_text(&[]), None, "nothing mapped");
        assert_eq!(mark_text(&[mark("CC 21")]).as_deref(), Some("CC 21"));
        assert_eq!(
            mark_text(&[mark("CC 21"), mark("Note 36"), mark("/1/fader1")]).as_deref(),
            Some("CC 21 +2"),
            "two more"
        );
    }

    /// A bank of one dropdown with `labels`.
    fn dropdown(labels: &[&str]) -> Result<ControlBank> {
        let values: Vec<usize> = (0..labels.len()).collect();
        let json = format!(
            r#"{{ "CONTROLS": [ {{ "NAME": "mode", "TYPE": "dropdown", "LABELS": {}, "VALUES": {} }} ] }}"#,
            serde_json::to_string(labels)?,
            serde_json::to_string(&values)?
        );
        let scene = SceneDef::parse(&json, "t")?;
        Ok(ControlBank::new(scene.controls)?)
    }

    /// Every piece of text in `shapes`, nested lists opened.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(inner) => {
                    for shape in inner {
                        walk(shape, out);
                    }
                }
                egui::Shape::Noop
                | egui::Shape::Rect(_)
                | egui::Shape::Circle(_)
                | egui::Shape::Ellipse(_)
                | egui::Shape::LineSegment { .. }
                | egui::Shape::Path(_)
                | egui::Shape::Band(_)
                | egui::Shape::Mesh(_)
                | egui::Shape::QuadraticBezier(_)
                | egui::Shape::CubicBezier(_)
                | egui::Shape::Callback(_) => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// One pass drawing the bank's first control with `marks`, at the page's
    /// scale; its row's rect and what was drawn.
    fn draw_first(
        ctx: &egui::Context,
        bank: &mut ControlBank,
        marks: &[Vec<Mark>],
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Rect, Vec<String>) {
        let theme = crate::palette::fallback().clone();
        let mut raw = crate::ui::testing::input(Vec2::new(400.0, 200.0), events);
        raw.time = Some(time);
        let mut rect = Rect::NOTHING;
        let pass = crate::ui::testing::run(ctx, raw, |ui| {
            let mut extras = Extras {
                pinned: &[],
                chosen: None,
                marks,
            };
            let mut extras = Some(&mut extras);
            rect = control(ui, &theme, bank, 0, &mut extras, Scale::Touch).unwrap_or(Rect::NOTHING);
        });
        (rect, texts(&pass.shapes))
    }

    #[test]
    fn a_short_dropdown_shows_its_options_and_a_click_chooses_one() -> Result<()> {
        let mut bank = dropdown(&["Off", "Soft", "Hard"])?;
        let ctx = egui::Context::default();
        let (row, drawn) = draw_first(&ctx, &mut bank, &[], 0.0, Vec::new());
        for label in ["Off", "Soft", "Hard"] {
            assert!(
                drawn.iter().any(|text| text == label),
                "{label} is drawn on the row: {drawn:?}"
            );
        }
        // The last option ends where the row's widgets end.
        let at = egui::pos2(row.right() - SPACE_SM - 8.0, row.center().y);
        let _ = draw_first(
            &ctx,
            &mut bank,
            &[],
            0.1,
            vec![egui::Event::PointerMoved(at), press(at, true)],
        );
        let _ = draw_first(&ctx, &mut bank, &[], 0.15, vec![press(at, false)]);
        let value = bank.target(0).copied().unwrap_or_default()[0];
        assert!(
            (value - 2.0).abs() < f32::EPSILON,
            "a click on the last option chose it: {value}"
        );
        Ok(())
    }

    #[test]
    fn a_long_dropdown_keeps_its_combo_box() -> Result<()> {
        let labels = ["One", "Two", "Three", "Four", "Five", "Six"];
        let mut bank = dropdown(&labels)?;
        let ctx = egui::Context::default();
        let (_, drawn) = draw_first(&ctx, &mut bank, &[], 0.0, Vec::new());
        assert!(
            drawn.iter().any(|text| text == "One"),
            "the chosen option shows in the box: {drawn:?}"
        );
        assert!(
            !drawn.iter().any(|text| text == "Six"),
            "the rest wait in the closed list: {drawn:?}"
        );
        Ok(())
    }

    #[test]
    fn a_mapped_control_shows_its_source_on_the_row() -> Result<()> {
        let json = r#"{ "CONTROLS": [ { "NAME": "swirl", "TYPE": "slider" } ] }"#;
        let scene = SceneDef::parse(json, "t")?;
        let mut bank = ControlBank::new(scene.controls)?;
        let marks = vec![vec![Mark {
            tag: "CC 21".to_string(),
            full: "MIDI CC 21 ch 1".to_string(),
        }]];
        let ctx = egui::Context::default();
        let (_, drawn) = draw_first(&ctx, &mut bank, &marks, 0.0, Vec::new());
        assert!(
            drawn.iter().any(|text| text == "CC 21"),
            "the mark is drawn: {drawn:?}"
        );
        let (_, unmapped) = draw_first(&ctx, &mut bank, &[], 0.1, Vec::new());
        assert!(
            !unmapped.iter().any(|text| text == "CC 21"),
            "no mark without a mapping: {unmapped:?}"
        );
        Ok(())
    }
}
