// ============================================================================
// Module:       strobe_app::ui::form
// Description:  The layout grammar for a page of settings: the page column,
//               the group card, the setting row, and the controls that sit
//               in a row (toggle switch, text field, slider).
//
// Dependencies: egui; super::{theme, widgets, motion}, crate::{icon, palette,
//               color}
// ============================================================================

//! How a page of settings is laid out (UI review section 4).
//!
//! The panes had widgets but no grammar: a label, then its control on the
//! next line, then a grey paragraph, at one type size, so every page read
//! as a wall of text. This module is the grammar, and every page of
//! settings is built from it:
//!
//! - [`page`]: a column no wider than [`MAX_WIDTH`], under the page's title,
//!   so a short form does not stretch across a projector-width window.
//! - [`group`]: a heading over a card, its rows divided by hairlines.
//! - [`Group::row`]: one setting. Its title (and a status chip) and a
//!   one-line description on the left; its control right-aligned in a fixed
//!   column. Narrower than [`STACK_BELOW`], the control drops under the
//!   text instead.
//! - [`Group::toggle`]: a row whose control is a [`toggle`] switch. A click
//!   anywhere on the row flips it, as a checkbox's label does.
//! - [`Group::block`]: content that is not one setting: a list, a banner, a
//!   pairing code.
//!
//! ## Controls sit right-aligned
//!
//! A row's control column lays out right to left, so the first widget a
//! closure adds is the rightmost. [`slider`] and [`field`] lay themselves out
//! left to right inside it, so a slider's number stays on its right.
//!
//! ## Revealing a row
//!
//! Search results ask for one row to be shown: [`reveal`] names it, and the
//! row with that title scrolls itself into view on its next draw and lights
//! up, fading over [`FLASH`] seconds.

use super::motion;
use super::theme::{self, RADIUS_LG, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::color::Rgb;
use crate::icon::Icon;
use crate::palette::Palette;
use egui::{Align, Layout, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::cell::RefCell;

/// The widest a page's column gets. Microsoft's settings guidance says about
/// 1000 px; a column of rows with a 300-point control column reads well
/// narrower than that, and the rest of a wide window stays empty rather
/// than stretching the gap between a title and its control.
pub const MAX_WIDTH: f32 = 760.0;

/// Below this row width the control goes under its text: side by side, the
/// text column would wrap a description into a word per line.
pub const STACK_BELOW: f32 = 460.0;

/// How long a revealed row stays lit, in seconds.
pub const FLASH: f64 = 1.6;

/// The most of a row's width an ordinary control column takes; the words
/// keep the rest.
const NARROW_SHARE: f32 = 0.45;

/// How many ordinary control columns wide a [`Group::wide_row`]'s is.
const WIDE: f32 = 1.45;

/// The most of a row's width a wide control column takes.
const WIDE_SHARE: f32 = 0.6;

/// A switch's track.
const SWITCH: Vec2 = Vec2::new(36.0, 20.0);

/// The width of a row's control column at the density in force.
#[must_use]
pub fn control_width() -> f32 {
    match theme::density() {
        theme::Density::Comfortable => 300.0,
        theme::Density::Compact => 260.0,
    }
}

/// The row asked to be shown, and when it was asked.
struct Reveal {
    title: String,
    since: Option<f64>,
}

thread_local! {
    /// The row [`reveal`] named, until its flash has faded.
    static REVEAL: RefCell<Option<Reveal>> = const { RefCell::new(None) };
}

/// Asks for the row titled `title` to scroll into view and light up the
/// next time it is drawn.
pub fn reveal(title: &str) {
    REVEAL.with(|reveal| {
        reveal.replace(Some(Reveal {
            title: title.to_owned(),
            since: None,
        }));
    });
}

/// How lit the row titled `title` is now, 0..=1, scrolling it into view the
/// first time it is asked about. Zero for every other row.
fn flash(ui: &Ui, title: &str, rect: Rect) -> f32 {
    let now = ui.input(|input| input.time);
    REVEAL.with(|reveal| {
        let mut reveal = reveal.borrow_mut();
        let Some(current) = reveal.as_mut().filter(|current| current.title == title) else {
            return 0.0;
        };
        let since = *current.since.get_or_insert_with(|| {
            ui.scroll_to_rect(rect, Some(Align::Center));
            now
        });
        let age = now - since;
        if age >= FLASH {
            *reveal = None;
            return 0.0;
        }
        ui.ctx().request_repaint();
        // Held for the first half, faded over the second.
        let fade = ((FLASH - age) / (FLASH / 2.0)).clamp(0.0, 1.0) as f32;
        motion::ease_out(fade)
    })
}

/// A page: the title, an optional line under it, and `add`'s groups, in a
/// column no wider than [`MAX_WIDTH`] at the left of the space.
pub fn page<R>(
    ui: &mut Ui,
    theme: &Palette,
    title: &str,
    lede: Option<&str>,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let width = ui.available_width().min(MAX_WIDTH);
    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        ui.label(
            egui::RichText::new(title)
                .font(theme::title_font())
                .color(theme::rgb(theme.text)),
        );
        if let Some(lede) = lede {
            ui.add_space(SPACE_XS);
            description(ui, lede, theme.text_muted);
        }
        add(ui)
    })
    .inner
}

/// A wrapped line of description text in `colour`.
fn description(ui: &mut Ui, text: &str, colour: Rgb) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::description_font())
                .color(theme::rgb(colour)),
        )
        .wrap(),
    );
}

/// One setting's words: its title, a status chip, what it does, and a note
/// (a failure, the app's own line) under that.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    title: &'a str,
    description: Option<&'a str>,
    status: Option<(Icon, &'a str, Rgb)>,
    note: Option<(&'a str, Rgb)>,
    enabled: bool,
}

impl<'a> Row<'a> {
    /// A row titled `title`.
    #[must_use]
    pub const fn new(title: &'a str) -> Self {
        Self {
            title,
            description: None,
            status: None,
            note: None,
            enabled: true,
        }
    }

    /// Whether the setting can be changed now. A disabled row is drawn
    /// faint, and its description should say why (Microsoft's guidance:
    /// never a dead control with no reason).
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// What the setting does, in a line under the title.
    #[must_use]
    pub const fn description(mut self, text: &'a str) -> Self {
        self.description = Some(text);
        self
    }

    /// A status chip after the title: an icon, a word, its colour.
    #[must_use]
    pub const fn status(mut self, icon: Icon, text: &'a str, colour: Rgb) -> Self {
        self.status = Some((icon, text, colour));
        self
    }

    /// A line under the description in `colour`: what the app says about
    /// the setting now.
    #[must_use]
    pub const fn note(mut self, text: &'a str, colour: Rgb) -> Self {
        self.note = Some((text, colour));
        self
    }

    /// Draws the words in a column `ui` has already sized.
    fn show(&self, ui: &mut Ui, theme: &Palette) {
        // The row's own layout zeroes the horizontal gap; the title and its
        // chip need one.
        ui.spacing_mut().item_spacing = Vec2::new(SPACE_SM, SPACE_XS / 2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(self.title).color(theme::rgb(theme.text)));
            if let Some((icon, text, colour)) = self.status {
                widgets::status_chip(ui, icon, text, colour);
            }
        });
        if let Some(text) = self.description {
            description(ui, text, theme.text_muted);
        }
        if let Some((text, colour)) = self.note {
            description(ui, text, colour);
        }
    }
}

/// A heading over a card of rows.
pub fn group<R>(
    ui: &mut Ui,
    theme: &Palette,
    heading: &str,
    add: impl FnOnce(&mut Group<'_>) -> R,
) -> R {
    ui.add_space(SPACE_LG);
    ui.label(
        egui::RichText::new(heading)
            .font(theme::heading_font())
            .color(theme::rgb(theme.text)),
    );
    ui.add_space(SPACE_SM);
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(egui::CornerRadius::same(RADIUS_LG))
        .inner_margin(theme::margin_xy(SPACE_MD, 0.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut group = Group { ui, theme, rows: 0 };
            add(&mut group)
        })
        .inner
}

/// A card of rows inside a group's [`Group::block`]: a row's options,
/// opened under it (Microsoft's settings expander), on the surface a step
/// darker than the group's card. Never nested deeper than this.
pub fn inset<R>(ui: &mut Ui, theme: &Palette, add: impl FnOnce(&mut Group<'_>) -> R) -> R {
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin_xy(SPACE_MD, 0.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut group = Group { ui, theme, rows: 0 };
            add(&mut group)
        })
        .inner
}

/// The rows of one [`group`]'s card.
pub struct Group<'u> {
    ui: &'u mut Ui,
    theme: &'u Palette,
    rows: usize,
}

impl Group<'_> {
    /// The `Ui` the rows go in, for what a row needs beyond its control:
    /// a scroll, a focus request.
    pub fn ui(&mut self) -> &mut Ui {
        self.ui
    }

    /// One setting, its control in a column [`control_width`] wide.
    pub fn row<R>(&mut self, row: Row<'_>, control: impl FnOnce(&mut Ui) -> R) -> R {
        self.layout(row, control_width(), NARROW_SHARE, control).0
    }

    /// One setting whose control is a path field with buttons beside it
    /// (Browse, Open): a column [`WIDE`] times as wide, taking up to
    /// [`WIDE_SHARE`] of the row.
    pub fn wide_row<R>(&mut self, row: Row<'_>, control: impl FnOnce(&mut Ui) -> R) -> R {
        self.layout(row, control_width() * WIDE, WIDE_SHARE, control)
            .0
    }

    /// One on/off setting. A click on the switch or anywhere on the row
    /// flips `on`; the response says `changed()` when it did.
    pub fn toggle(&mut self, row: Row<'_>, on: &mut bool) -> Response {
        let theme = self.theme;
        let (switch, rect) = self.layout(row, SWITCH.x, NARROW_SHARE, |ui| toggle(ui, theme, on));
        // Registered after the switch, so a pointer click lands here
        // wherever it is on the row; the switch still answers Space when
        // it has the focus. Not focusable itself: one tab stop per setting.
        let whole = self.ui.interact(
            rect,
            self.ui.scope_id().with(("setting-row", row.title)),
            Sense::CLICK,
        );
        let mut switch = switch;
        if whole.clicked() && row.enabled {
            *on = !*on;
            switch.mark_changed();
        }
        let whole = if whole.enabled() {
            whole.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            whole
        };
        switch.union(whole)
    }

    /// Content that is not one setting, the card's full width.
    pub fn block<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.divide();
        self.ui.add_space(theme::row_pad());
        let inner = self
            .ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = SPACE_XS;
                add(ui)
            })
            .inner;
        self.ui.add_space(theme::row_pad());
        self.rows += 1;
        inner
    }

    /// A hairline above every row but the first.
    fn divide(&mut self) {
        if self.rows == 0 {
            return;
        }
        let width = self.ui.available_width();
        let (rect, _) = self
            .ui
            .allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
        self.ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            Stroke::new(1.0, theme::rgb(self.theme.border)),
        );
    }

    /// Lays one row out; returns the control's result and the row's rect.
    fn layout<R>(
        &mut self,
        row: Row<'_>,
        control_width: f32,
        share: f32,
        control: impl FnOnce(&mut Ui) -> R,
    ) -> (R, Rect) {
        self.divide();
        let theme = self.theme;
        let ui = &mut *self.ui;
        // Painted behind the row once its size is known.
        let backdrop = ui.painter().add(egui::Shape::Noop);
        let top = ui.cursor().top();
        ui.add_space(theme::row_pad());
        let width = ui.available_width();
        let inner = ui.add_enabled_ui(row.enabled, |ui| {
            Self::arrange(
                ui,
                theme,
                &row,
                width,
                control_width.min(width * share),
                control,
            )
        });
        let inner = inner.inner;
        ui.add_space(theme::row_pad());
        let rect = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top().max(top));
        let lit = flash(ui, row.title, rect);
        if lit > 0.0 {
            let alpha = (lit * 60.0).round().clamp(0.0, 255.0) as u8;
            ui.painter().set(
                backdrop,
                egui::Shape::rect_filled(
                    rect.expand2(Vec2::new(SPACE_MD, 0.0)),
                    egui::CornerRadius::ZERO,
                    theme::translucent(theme.accent, alpha),
                ),
            );
        }
        self.rows += 1;
        (inner, rect)
    }

    /// Places a row's words and control in `width`: side by side, or the
    /// control under the words when the row is narrow.
    fn arrange<R>(
        ui: &mut Ui,
        theme: &Palette,
        row: &Row<'_>,
        width: f32,
        control_width: f32,
        control: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        if width < STACK_BELOW {
            ui.scope(|ui| {
                ui.set_width(width);
                row.show(ui, theme);
            });
            ui.add_space(SPACE_SM);
            ui.allocate_ui_with_layout(
                Vec2::new(width, 0.0),
                Layout::left_to_right(Align::Center).with_main_wrap(true),
                |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(SPACE_SM, SPACE_XS);
                    control(ui)
                },
            )
            .inner
        } else {
            // The gap between the words and the control is the scale's largest step.
            let text_width = width - control_width - SPACE_LG;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let text = ui
                    .allocate_ui_with_layout(
                        Vec2::new(text_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(text_width);
                            row.show(ui, theme);
                        },
                    )
                    .response
                    .rect;
                ui.add_space(SPACE_LG);
                // At least a button's height, so a lone label or chip in the
                // column centres on the same line as the buttons beside it.
                let height = text
                    .height()
                    .max(ui.spacing().interact_size.y)
                    .max(widgets::BUTTON_HEIGHT);
                ui.allocate_ui_with_layout(
                    Vec2::new(control_width, height),
                    // Wrapping: controls wider than the column take a second
                    // line inside it rather than spilling left over the words.
                    Layout::right_to_left(Align::Center).with_main_wrap(true),
                    |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(SPACE_SM, SPACE_XS);
                        ui.set_min_size(Vec2::new(control_width, height));
                        control(ui)
                    },
                )
                .inner
            })
            .inner
        }
    }
}

/// An on/off switch: a track that fills with the accent when on and a knob
/// that slides across. Flips `on` when clicked, or on Space or Enter when
/// it has the focus.
pub fn toggle(ui: &mut Ui, theme: &Palette, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(SWITCH, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let state = *on;
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, state, ""));
    let t = motion::toggle(ui.ctx(), response.id, state, motion::QUICK);
    let painter = ui.painter();
    let radius = egui::CornerRadius::same((SWITCH.y / 2.0) as u8);
    let off_track = widgets::hover_fill(
        ui,
        response.id.with("hover"),
        response.hovered(),
        theme.control,
        theme.control_hover,
    );
    let track = if state {
        theme::rgb(theme.accent)
    } else {
        off_track
    };
    painter.rect_filled(rect, radius, track);
    if !state {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::rgb(theme.border_strong)),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            egui::CornerRadius::same((SWITCH.y / 2.0 + 2.0) as u8),
            Stroke::new(2.0, theme::rgb(theme.accent_hover)),
            egui::StrokeKind::Outside,
        );
    }
    let knob_radius = SWITCH.y / 2.0 - 3.0;
    let left = rect.left() + 3.0 + knob_radius;
    let right = rect.right() - 3.0 - knob_radius;
    let knob = egui::pos2(left + (right - left) * t, rect.center().y);
    let ink = if state {
        theme.text_on_accent
    } else {
        theme.text_muted
    };
    painter.circle_filled(knob, knob_radius, theme::rgb(ink));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A single-line text field `width` wide: outlined, with room inside, and
/// a placeholder fainter than any value so an empty field never reads as a
/// filled one (UI review S3).
pub fn field(ui: &mut Ui, theme: &Palette, text: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(egui::RichText::new(hint).color(theme::rgb(theme.text_faint)))
            .desired_width(width)
            .margin(theme::margin_xy(SPACE_SM, SPACE_XS)),
    )
}

/// A slider filling the control column, its value on its right formatted
/// by `format`.
pub fn slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: Option<f64>,
    format: impl Fn(f64) -> String,
) -> Response {
    /// Room for the value box beside the track.
    const VALUE: f32 = 64.0;
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - VALUE).max(SPACE_LG * 3.0);
        let mut slider = egui::Slider::new(value, range).custom_formatter(|v, _| format(v));
        if let Some(step) = step {
            slider = slider.step_by(step);
        }
        ui.add(slider)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn texts(pass: &crate::ui::testing::Pass) -> Vec<(String, Rect)> {
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

    fn find(drawn: &[(String, Rect)], text: &str) -> Result<Rect> {
        drawn
            .iter()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| *rect)
            .ok_or_else(|| anyhow::anyhow!("no {text:?} in {drawn:?}"))
    }

    fn one_row(ctx: &egui::Context, width: f32, on: &mut bool) -> crate::ui::testing::Pass {
        let theme = crate::palette::fallback().clone();
        crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(width, 400.0), Vec::new()),
            |ui| {
                group(ui, &theme, "Heading", |group| {
                    group.toggle(
                        Row::new("Switch title").description("What the switch does."),
                        on,
                    );
                    group.row(Row::new("Field title"), |ui| {
                        ui.label("CONTROL");
                    });
                });
            },
        )
    }

    #[test]
    fn a_wide_row_puts_its_control_right_of_its_words() -> Result<()> {
        let ctx = egui::Context::default();
        let pass = one_row(&ctx, 700.0, &mut false);
        let drawn = texts(&pass);
        let title = find(&drawn, "Field title")?;
        let control = find(&drawn, "CONTROL")?;
        assert!(
            control.left() > title.right() + SPACE_LG,
            "the control sits in its own column to the right: {title:?} {control:?}"
        );
        assert!(
            control.center().y < title.top() + widgets::BUTTON_HEIGHT,
            "and level with the title, not below it: {title:?} {control:?}"
        );
        Ok(())
    }

    #[test]
    fn a_narrow_row_puts_its_control_under_its_words() -> Result<()> {
        let ctx = egui::Context::default();
        let pass = one_row(&ctx, STACK_BELOW - 40.0, &mut false);
        let drawn = texts(&pass);
        let title = find(&drawn, "Field title")?;
        let control = find(&drawn, "CONTROL")?;
        assert!(
            control.top() >= title.bottom(),
            "the control drops below the title: {title:?} {control:?}"
        );
        Ok(())
    }

    #[test]
    fn a_click_on_a_toggle_rows_words_flips_it() -> Result<()> {
        let ctx = egui::Context::default();
        let mut on = false;
        let drawn = texts(&one_row(&ctx, 700.0, &mut on));
        let at = find(&drawn, "What the switch does.")?.center();
        let theme = crate::palette::fallback().clone();
        let mut changed = false;
        for events in [
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            crate::ui::testing::run(
                &ctx,
                crate::ui::testing::input(Vec2::new(700.0, 400.0), events),
                |ui| {
                    group(ui, &theme, "Heading", |group| {
                        changed |= group
                            .toggle(
                                Row::new("Switch title").description("What the switch does."),
                                &mut on,
                            )
                            .changed();
                    });
                },
            );
        }
        assert!(on, "a click on the description flips the switch");
        assert!(changed, "and the response says so");
        Ok(())
    }
}
