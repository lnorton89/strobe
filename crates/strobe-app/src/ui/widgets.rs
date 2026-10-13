// ============================================================================
// Module:       strobe_app::ui::widgets
// Description:  The shared vocabulary every pane is built from: rail items,
//               tool and transport buttons, chips, banners, pane headers,
//               meters, empty states, and the hover ramp.
//
// Dependencies: egui; super::{theme, icon, motion}, crate::icon, crate::palette.
//               Ported and adapted from rustaman's `gui::ui::widgets` (D5, D130).
// ============================================================================

//! The widgets every pane shares.
//!
//! Building each pane out of raw `ui.label` and `ui.button` calls produces
//! an app where one pane's chips are two pixels taller than another's and
//! nobody can say why. Everything with a repeated shape is a function here.
//!
//! ## Every hover fades; nothing switches
//!
//! Hoverable surfaces route their highlight through [`hover_fill`], one
//! eased ramp over [`crate::motion::INSTANT`] for the whole window. One
//! control that snaps beside one that fades is most of what reads as
//! unfinished. (Under reduced motion the ramp has zero length.)
//!
//! ## Icons are geometry
//!
//! Nothing here sets an icon in a font; see [`crate::icon`].
//!
//! ## State is never colour alone
//!
//! A badge, chip or banner that carries a state always carries an icon and
//! a word as well as a colour (UI_DESIGN section 27).
//!
//! ## Measure, then allocate, then paint
//!
//! An `egui::Frame` does not wrap: it measures itself against the space left
//! on the current line and *then* allocates what it measured. Every
//! chip-shaped widget here therefore measures its text, allocates that size
//! exactly with [`egui::Ui::allocate_exact_size`], and paints into the rect
//! it gets back.

use super::icon as icons;
use super::motion;
use super::theme::{self, RADIUS, SELECTION_BAR, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::color::Rgb;
use crate::icon::Icon;
use crate::palette::Palette;
use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, TextStyle,
    Ui, Vec2,
};

/// How far a hover animation has progressed, 0..=1, eased.
#[must_use]
pub fn hover_t(ui: &Ui, id: egui::Id, hovered: bool) -> f32 {
    motion::hover(ui, id, hovered)
}

/// The fill for a hoverable surface, part-way through its ramp.
#[must_use]
pub fn hover_fill(ui: &Ui, id: egui::Id, hovered: bool, rest: Rgb, lifted: Rgb) -> Color32 {
    let t = hover_t(ui, id, hovered);
    theme::rgb(rest.lerp(lifted, t))
}

/// Where a selection bar sits, which follows from what it marks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bar {
    /// A list row, which runs from one edge of its pane to the other.
    Flush,
    /// A rounded card or rail entry: inset, because a square bar drawn
    /// flush inside a rounded card pokes its corners out through the card's.
    Inset,
}

/// The bar's corner radius in the [`Bar::Inset`] case.
const BAR_RADIUS: u8 = 2;

impl Bar {
    fn inset(self) -> f32 {
        match self {
            Self::Flush => 0.0,
            Self::Inset => SPACE_XS,
        }
    }

    fn radius(self) -> CornerRadius {
        match self {
            Self::Flush => CornerRadius::ZERO,
            Self::Inset => CornerRadius::same(BAR_RADIUS),
        }
    }
}

/// Paints the bar down the leading edge of a selected row or card. Every
/// selection marker in the app comes through here. It grows out from the
/// centre rather than appearing whole, so moving a selection down a list
/// reads as one marker travelling. **Key `id` on the thing selected**, not
/// on its position.
pub fn accent_bar(
    ui: &Ui,
    painter: &egui::Painter,
    id: egui::Id,
    rect: Rect,
    bar: Bar,
    color: Rgb,
) {
    let grown = motion::toggle(ui.ctx(), id.with("selected"), true, motion::QUICK);
    let height = (rect.height() - bar.inset() * 2.0).max(0.0) * grown;
    let painted = Rect::from_center_size(
        egui::pos2(rect.left() + SELECTION_BAR / 2.0, rect.center().y),
        Vec2::new(SELECTION_BAR, height),
    );
    painter.rect_filled(painted, bar.radius(), theme::rgb(color));
}

/// The fill behind a selectable row or card.
#[must_use]
pub fn selection_fill(
    ui: &Ui,
    theme: &Palette,
    id: egui::Id,
    selected: bool,
    hovered: bool,
    rest: Rgb,
) -> Color32 {
    if selected {
        theme::rgb(theme.selection)
    } else {
        hover_fill(ui, id, hovered, rest, theme.hover)
    }
}

/// A navigation-rail entry: an icon over a short label, and an accent bar
/// when active. A disabled entry (a workspace that is not built yet) is
/// drawn faint and says why in its tooltip. With `show_label` false only the
/// icon remains (a narrow window).
pub fn nav_item(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
    show_label: bool,
) -> Response {
    let height = if show_label { 50.0 } else { 38.0 };
    let width = ui.available_width();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    let painter = ui.painter();

    let id = egui::Id::new(("nav", label));
    let fill = selection_fill(
        ui,
        theme,
        response.id,
        active,
        enabled && response.hovered(),
        theme.app,
    );
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    if active {
        accent_bar(ui, painter, id, rect, Bar::Inset, theme.accent);
    }

    let colour = if !enabled {
        theme.text_faint
    } else if active {
        theme.accent
    } else {
        theme.text_muted
    };
    let icon_centre = if show_label {
        egui::pos2(rect.center().x, rect.top() + 18.0)
    } else {
        rect.center()
    };
    icons::paint(
        painter,
        Rect::from_center_size(icon_centre, Vec2::splat(icons::NAV)),
        icon,
        theme::rgb(colour),
    );
    if show_label {
        painter.text(
            egui::pos2(rect.center().x, rect.bottom() - 9.0),
            Align2::CENTER_CENTER,
            label,
            FontId::new(10.0, egui::FontFamily::Proportional),
            theme::rgb(if active { theme.text } else { colour }),
        );
    }
    response
}

/// A borderless square icon button, for the title bar and toolbars.
pub fn icon_button(ui: &mut Ui, theme: &Palette, icon: Icon, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let fill = hover_fill(ui, response.id, response.hovered(), theme.app, theme.hover);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), fill);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(icons::INLINE)),
        icon,
        theme::rgb(theme.text),
    );
    response.on_hover_text(tooltip)
}

/// A window-control button for the custom title bar. The close variant
/// highlights in the danger colour, as every Windows window does.
pub fn window_button(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    tooltip: &str,
    closes: bool,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(40.0, theme::TITLE_BAR_HEIGHT), Sense::click());
    let lifted = if closes { theme.danger } else { theme.hover };
    let fill = hover_fill(ui, response.id, response.hovered(), theme.app, lifted);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    // The mark flips to the on-accent colour as the danger fill comes up, or
    // it goes unreadable against it at the end of the ramp.
    let t = hover_t(ui, response.id, response.hovered());
    let mark = if closes {
        theme.text.lerp(theme.text_on_accent, t)
    } else {
        theme.text
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(icons::INLINE)),
        icon,
        theme::rgb(mark),
    );
    response.on_hover_text(tooltip)
}

/// A small tool button: an icon and a label, tinted when `active`.
pub fn tool_button(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    label: &str,
    active: bool,
    tooltip: &str,
) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let colour = if active { theme.accent } else { theme.text };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, theme::rgb(colour));
    let width = SPACE_SM * 2.0 + icons::INLINE + SPACE_XS + galley.size().x;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let rest = if active {
        theme.accent_soft
    } else {
        theme.raised
    };
    let lifted = if active {
        theme.accent_soft
    } else {
        theme.hover
    };
    let fill = hover_fill(ui, response.id, response.hovered(), rest, lifted);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    if active {
        painter.rect_stroke(
            rect,
            CornerRadius::same(RADIUS),
            Stroke::new(1.0, theme::rgb(theme.accent)),
            StrokeKind::Inside,
        );
    }
    icons::paint(
        painter,
        Rect::from_center_size(
            egui::pos2(
                rect.left() + SPACE_SM + icons::INLINE / 2.0,
                rect.center().y,
            ),
            Vec2::splat(icons::INLINE),
        ),
        icon,
        theme::rgb(colour),
    );
    painter.galley(
        egui::pos2(
            rect.left() + SPACE_SM + icons::INLINE + SPACE_XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::rgb(colour),
    );
    response.on_hover_text(tooltip)
}

/// How a transport button is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// Ready to press.
    Normal,
    /// Latched on (a held cue).
    Active,
    /// Not wired to anything yet; drawn faint, with a dashed outline so it
    /// reads as absent rather than merely quiet.
    Unavailable,
}

/// A transport button: a large square with a mark in it.
pub fn transport_button(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    look: Look,
    tooltip: &str,
) -> Response {
    let sense = match look {
        Look::Unavailable => Sense::hover(),
        Look::Normal | Look::Active => Sense::click(),
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme::TRANSPORT_BUTTON), sense);
    let (rest, lifted, mark) = match look {
        Look::Normal => (theme.raised, theme.hover, theme.text),
        Look::Active => (theme.accent_soft, theme.accent_soft, theme.accent),
        Look::Unavailable => (theme.panel, theme.panel, theme.text_faint),
    };
    let fill = hover_fill(ui, response.id, response.hovered(), rest, lifted);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    let outline = match look {
        Look::Active => theme.accent,
        Look::Normal | Look::Unavailable => theme.border,
    };
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, theme::rgb(outline)),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(icons::TRANSPORT)),
        icon,
        theme::rgb(mark),
    );
    response.on_hover_text(tooltip)
}

/// The blackout button: a distinct danger-outlined control that fills when
/// the output is blacked out, and says so in words.
pub fn blackout_button(ui: &mut Ui, theme: &Palette, on: bool, tooltip: &str) -> Response {
    let label = if on { "BLACKOUT ON" } else { "BLACKOUT" };
    let font = FontId::new(13.0, egui::FontFamily::Proportional);
    let colour = if on {
        theme.text_on_accent
    } else {
        theme.danger
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, theme::rgb(colour));
    let width = SPACE_MD + icons::INLINE + SPACE_XS + galley.size().x + SPACE_MD;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::TRANSPORT_BUTTON), Sense::click());
    let rest = if on { theme.danger } else { theme.panel };
    let lifted = if on { theme.danger } else { theme.raised };
    let fill = hover_fill(ui, response.id, response.hovered(), rest, lifted);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(if on { 2.0 } else { 1.5 }, theme::rgb(theme.danger)),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(
            egui::pos2(
                rect.left() + SPACE_MD + icons::INLINE / 2.0,
                rect.center().y,
            ),
            Vec2::splat(icons::INLINE),
        ),
        Icon::Blackout,
        theme::rgb(colour),
    );
    painter.galley(
        egui::pos2(
            rect.left() + SPACE_MD + icons::INLINE + SPACE_XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::rgb(colour),
    );
    response.on_hover_text(tooltip)
}

/// A primary (accent-filled) button.
pub fn primary_button(ui: &mut Ui, theme: &Palette, label: &str) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), font, theme::rgb(theme.text_on_accent));
    let size = galley.size() + Vec2::new(SPACE_MD * 2.0, SPACE_SM);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.accent,
        theme.accent_hover,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(theme.text_on_accent),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A secondary button: raised surface, outlined, an optional leading icon.
pub fn button(ui: &mut Ui, theme: &Palette, icon: Option<Icon>, label: &str) -> Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, theme::rgb(theme.text));
    let lead = if icon.is_some() {
        icons::INLINE + SPACE_XS
    } else {
        0.0
    };
    let size = Vec2::new(galley.size().x + lead + SPACE_MD * 2.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.raised,
        theme.hover,
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + SPACE_MD;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_center_size(
                egui::pos2(x + icons::INLINE / 2.0, rect.center().y),
                Vec2::splat(icons::INLINE),
            ),
            icon,
            theme::rgb(theme.text),
        );
        x += lead;
    }
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::rgb(theme.text),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small labelled pill: a count, a source badge.
pub fn chip(ui: &mut Ui, text: &str, fill: Rgb, text_color: Rgb) -> Response {
    let font = TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, theme::rgb(text_color));
    let size = galley.size() + Vec2::new(SPACE_SM * 2.0, SPACE_XS * 1.5);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS), theme::rgb(fill));
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(text_color),
    );
    response
}

/// A chip carrying a **status**: an icon, a word, and the status colour as a
/// wash behind them.
pub fn status_chip(ui: &mut Ui, icon: Icon, text: &str, color: Rgb) -> Response {
    status_chip_over(ui, icon, text, color, None)
}

/// [`status_chip`] for a chip that sits over a picture: `plate` is the
/// colour of an opaque-ish backing that keeps the words readable whatever
/// the picture is doing.
pub fn status_chip_over(
    ui: &mut Ui,
    icon: Icon,
    text: &str,
    color: Rgb,
    plate: Option<Rgb>,
) -> Response {
    /// How strongly the status colour tints the pill: a wash behind text
    /// drawn in the same hue at full strength.
    const TINT: u8 = 38;
    let font = TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, theme::rgb(color));
    let glyph = icons::DISCLOSURE;
    let size = Vec2::new(
        galley.size().x + glyph + SPACE_XS + SPACE_SM * 2.0,
        galley.size().y + SPACE_XS * 1.5,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    if let Some(plate) = plate {
        painter.rect_filled(
            rect,
            CornerRadius::same(RADIUS),
            theme::translucent(plate, 224),
        );
    }
    painter.rect_filled(
        rect,
        CornerRadius::same(RADIUS),
        theme::translucent(color, TINT),
    );
    icons::paint(
        painter,
        Rect::from_center_size(
            egui::pos2(rect.left() + SPACE_SM + glyph / 2.0, rect.center().y),
            Vec2::splat(glyph),
        ),
        icon,
        theme::rgb(color),
    );
    painter.galley(
        egui::pos2(
            rect.left() + SPACE_SM + glyph + SPACE_XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::rgb(color),
    );
    response
}

/// Lays a single line of text out, truncated with an ellipsis if it does not
/// fit `width`.
pub fn truncated(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Rgb,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(font, theme::rgb(color)),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// A scene's identity colour: stable for a name, whatever its position in
/// the list, so a scene keeps one swatch in the browser and on the cue
/// strip (UI_DESIGN section 19). FNV-1a of the name into the theme's ramp.
#[must_use]
pub fn scene_colour(theme: &Palette, name: &str) -> Rgb {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    theme.series_for(hash)
}

/// A pane's header: icon, title and a close button, over a rule. Anything
/// the pane wants on the right goes in `extra`. Returns whether the close
/// button was pressed.
pub fn pane_header(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    title: &str,
    extra: impl FnOnce(&mut Ui),
) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        ui.set_height(theme::PANE_HEADER);
        icons::show(ui, icon, icons::INLINE, theme::rgb(theme.accent));
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .small()
                .strong()
                .color(theme::rgb(theme.text)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, theme, Icon::Close, "Close (Esc)").clicked() {
                closed = true;
            }
            extra(ui);
        });
    });
    section_rule(ui, theme);
    ui.add_space(SPACE_SM);
    closed
}

/// A one-pixel rule the width of the space available.
pub fn section_rule(ui: &mut Ui, theme: &Palette) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, theme::rgb(theme.border)),
    );
}

/// A small-caps section heading with its rule.
pub fn section(ui: &mut Ui, theme: &Palette, title: &str) {
    ui.add_space(SPACE_XS);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(10.5)
            .strong()
            .color(theme::rgb(theme.text_muted)),
    );
    ui.add_space(SPACE_XS);
}

/// A labelled fraction bar: GPU time against its budget, say.
pub fn meter(ui: &mut Ui, theme: &Palette, fraction: f32, fill: Rgb, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    let radius = CornerRadius::same(3);
    ui.painter()
        .rect_filled(rect, radius, theme::rgb(theme.raised));
    // A non-finite fraction would produce a NaN width, which egui paints as
    // nothing: a broken measurement would look like an empty bar.
    let target = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fraction = motion::value(ui.ctx(), response.id.with("level"), target);
    if fraction > 0.0 {
        let filled = Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * fraction).max(2.0), rect.height()),
        );
        ui.painter().rect_filled(filled, radius, theme::rgb(fill));
    }
    response
}

/// An empty-state message centred in the space left: an icon, a title and a
/// line of detail. A pane with nothing in it has to say so, and say what to
/// do; a blank pane is indistinguishable from one that failed to load.
pub fn empty_state(ui: &mut Ui, theme: &Palette, icon: Icon, title: &str, detail: &str) {
    let rect = ui.available_rect_before_wrap();
    let centre = rect.center();
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(centre - Vec2::new(0.0, 34.0), Vec2::splat(28.0)),
        icon,
        theme::rgb(theme.text_faint),
    );
    painter.text(
        centre - Vec2::new(0.0, 6.0),
        Align2::CENTER_CENTER,
        title,
        FontId::new(15.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text),
    );
    // Wrapped, not truncated: the detail says what to do next, and an
    // instruction cut off mid-word is worse than none.
    let galley = ui.painter().layout(
        detail.to_string(),
        TextStyle::Small.resolve(ui.style()),
        theme::rgb(theme.text_muted),
        (rect.width() - SPACE_LG_X2).max(40.0),
    );
    ui.painter().galley(
        egui::pos2(centre.x - galley.size().x / 2.0, centre.y + 12.0),
        galley,
        theme::rgb(theme.text_muted),
    );
    ui.allocate_space(rect.size());
}

/// Twice [`theme::SPACE_LG`], the margin an empty state keeps from the pane
/// edge on both sides.
const SPACE_LG_X2: f32 = theme::SPACE_LG * 2.0;

/// What a banner is saying.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Informational.
    Info,
    /// Something degraded but the show continues.
    Warning,
    /// Something failed.
    Error,
}

impl Tone {
    /// The colour of the tone in `theme`.
    #[must_use]
    pub fn colour(self, theme: &Palette) -> Rgb {
        match self {
            Self::Info => theme.info,
            Self::Warning => theme.warning,
            Self::Error => theme.danger,
        }
    }

    /// The icon that accompanies the tone, so it is never colour alone.
    #[must_use]
    pub fn icon(self) -> Icon {
        match self {
            Self::Info => Icon::Info,
            Self::Warning => Icon::Warning,
            Self::Error => Icon::Error,
        }
    }

    /// The word that accompanies the tone.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Info => "NOTICE",
            Self::Warning => "WARNING",
            Self::Error => "FAILED",
        }
    }
}

/// A banner: an icon, a tone word, a message, and buttons on the right.
/// Used for the things that must be visible without being a modal: a scene
/// that failed, a recovered context.
pub fn banner(
    ui: &mut Ui,
    theme: &Palette,
    tone: Tone,
    message: &str,
    buttons: impl FnOnce(&mut Ui),
) {
    /// Below this width the message gets its own wrapped row instead of
    /// sharing the first one with the tone word and the buttons.
    const WIDE: f32 = 520.0;
    let colour = tone.colour(theme);
    let wide = ui.available_width() >= WIDE;
    egui::Frame::new()
        .fill(theme::translucent(colour, 28))
        .stroke(Stroke::new(1.0, theme::rgb(colour)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(theme::margin_xy(SPACE_SM, SPACE_XS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icons::show(ui, tone.icon(), icons::INLINE, theme::rgb(colour));
                ui.label(
                    egui::RichText::new(tone.word())
                        .small()
                        .strong()
                        .color(theme::rgb(colour)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    buttons(ui);
                    if wide {
                        let width = ui.available_width();
                        let galley = truncated(
                            ui,
                            message,
                            TextStyle::Body.resolve(ui.style()),
                            theme.text,
                            width,
                        );
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(width.min(galley.size().x), galley.size().y),
                            Sense::hover(),
                        );
                        ui.painter()
                            .galley(rect.left_top(), galley, theme::rgb(theme.text));
                        response.on_hover_text(message);
                    }
                });
            });
            if !wide {
                ui.label(egui::RichText::new(message).color(theme::rgb(theme.text)));
            }
        });
}

/// A key/value line for a details pane, painted so both columns share a
/// baseline.
pub fn kv_row(ui: &mut Ui, theme: &Palette, key: &str, value: &str, value_colour: Rgb) {
    /// The label column's width, so every value starts on one column.
    const KEY_WIDTH: f32 = 96.0;
    let height = ui.text_style_height(&TextStyle::Small);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height + SPACE_XS),
        Sense::hover(),
    );
    let font = TextStyle::Small.resolve(ui.style());
    ui.painter().galley(
        egui::pos2(rect.left(), rect.top()),
        truncated(ui, key, font.clone(), theme.text_muted, KEY_WIDTH),
        theme::rgb(theme.text_muted),
    );
    ui.painter().galley(
        egui::pos2(rect.left() + KEY_WIDTH, rect.top()),
        truncated(ui, value, font, value_colour, rect.width() - KEY_WIDTH),
        theme::rgb(value_colour),
    );
    response.on_hover_text(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Catalog;

    #[test]
    fn every_tone_has_a_distinct_icon_and_word() {
        let tones = [Tone::Info, Tone::Warning, Tone::Error];
        for (i, a) in tones.iter().enumerate() {
            for b in tones.iter().skip(i + 1) {
                assert!(a.icon() != b.icon(), "{a:?} and {b:?} share an icon");
                assert!(a.word() != b.word(), "{a:?} and {b:?} share a word");
            }
        }
    }

    #[test]
    fn every_tone_is_visible_against_every_themes_panel() {
        for theme in Catalog::load().themes() {
            for tone in [Tone::Info, Tone::Warning, Tone::Error] {
                let ratio = tone.colour(theme).contrast(theme.panel);
                assert!(
                    ratio >= 3.0,
                    "{}: {tone:?} is {ratio:.2}:1 against the panel",
                    theme.name
                );
            }
        }
    }

    /// Draws `f` in a headless egui pass and returns the size of what it
    /// allocated, so a widget's measured size can be tested without a GPU.
    fn size_of(f: impl FnOnce(&mut Ui, &Palette)) -> Vec2 {
        let ctx = egui::Context::default();
        let theme = crate::palette::fallback().clone();
        theme::apply(&ctx, &theme, theme::Density::default());
        let mut size = Vec2::ZERO;
        let mut f = Some(f);
        let _ = crate::ui::testing::run(
            &ctx,
            crate::ui::testing::input(Vec2::new(800.0, 600.0), Vec::new()),
            |ui| {
                let before = ui.min_rect();
                if let Some(f) = f.take() {
                    f(ui, &theme);
                }
                size = ui.min_rect().size() - before.size();
            },
        );
        size
    }

    #[test]
    fn a_scene_keeps_its_colour_and_different_scenes_differ() {
        let theme = crate::palette::fallback();
        assert_eq!(
            scene_colour(theme, "aurora"),
            scene_colour(theme, "aurora"),
            "stable"
        );
        let names = [
            "aurora",
            "plasma",
            "tunnel",
            "05-domain-warp",
            "11-script-orbits",
        ];
        let colours: Vec<Rgb> = names.iter().map(|n| scene_colour(theme, n)).collect();
        for (i, a) in colours.iter().enumerate() {
            for b in colours.iter().skip(i + 1) {
                assert!(a != b, "two scenes share {a:?}");
            }
        }
    }

    #[test]
    fn a_transport_button_is_big_enough_to_hit_on_stage() {
        let mut got = Vec2::ZERO;
        let _ = size_of(|ui, theme| {
            let response = transport_button(ui, theme, Icon::Play, Look::Normal, "Play");
            got = response.rect.size();
        });
        assert!(
            got.x >= 32.0 && got.y >= 32.0,
            "a transport button is {got:?}"
        );
    }

    #[test]
    fn a_nonsense_meter_fraction_draws_an_empty_bar_not_a_nan_one() {
        let mut width = f32::NAN;
        let _ = size_of(|ui, theme| {
            let response = meter(ui, theme, f32::NAN, theme.accent, 100.0);
            width = response.rect.width();
        });
        assert!(width.is_finite() && width > 0.0, "meter width {width}");
    }
}
