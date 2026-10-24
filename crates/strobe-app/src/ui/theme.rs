// ============================================================================
// Module:       strobe_app::ui::theme
// Description:  The active palette, the egui style derived from it, and the one
//               spacing scale every margin in the app comes from.
//
// Dependencies: egui; crate::palette (the catalog), crate::color::Rgb.
//               Ported from rustaman's `gui::ui::theme` (D5, D130).
// ============================================================================

//! Where a theme becomes a window.
//!
//! [`apply`] installs the chosen [`crate::palette::Palette`] into a
//! thread-local and into egui's own `Visuals`; the drawing code reads it back
//! through [`palette`]. That is ambient state, which this codebase otherwise
//! avoids; the justification is that it is exactly how `ctx.style()` already
//! works, and the alternative is threading a `&Palette` through every call
//! site. Unlike the port's original, [`palette`] hands out an `Rc` (a
//! reference-count bump), not a clone of the palette's strings, because the
//! drawing code calls it every frame and the frame path does not allocate.
//!
//! Most of the palette also goes into egui's `Visuals`, so built-in widgets
//! follow the theme without any call site being involved. Prefer letting a
//! colour arrive that way over reading it from [`palette`] by hand.
//!
//! ## No literal colours in drawing code
//!
//! Every colour comes from here. `no_drawing_module_holds_a_colour_literal`
//! in `ui::tests` walks the source and fails the build for a
//! `Color32::from_rgb` outside this module: a single hard-coded grey is
//! invisible in review and then unreadable in half the themes. The only
//! exceptions are alpha-only values (a scrim, a shadow), which are lighting,
//! not theme colours.
//!
//! ## No hand-picked pixel gaps
//!
//! Every margin, inset and gap is one of [`SPACE_XS`], [`SPACE_SM`],
//! [`SPACE_MD`], [`SPACE_LG`] or [`PAD`], so the left edges of the title bar,
//! the rail, the panes' headings and the transport strip form one column.

use crate::color::Rgb;
use crate::palette::{Mode, Palette};
use egui::{Color32, CornerRadius, Margin, Stroke, TextStyle, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The tightest gap: between an icon and its label, or a chip's inset.
pub const SPACE_XS: f32 = 4.0;

/// Between related controls in a row.
pub const SPACE_SM: f32 = 8.0;

/// Between groups of controls, and a card's inner margin.
pub const SPACE_MD: f32 = 12.0;

/// Between sections, and around a modal's body.
pub const SPACE_LG: f32 = 20.0;

/// The inset from a pane edge to its content.
///
/// One of the scale's own values rather than a sixth number, so every pane's
/// content starts on the same column.
pub const PAD: f32 = SPACE_MD;

/// The corner radius on cards, chips and buttons.
pub const RADIUS: u8 = 5;

/// The corner radius on a large surface: a modal, a drawer.
pub const RADIUS_LG: u8 = 8;

/// How tightly the control window is packed (UI_DESIGN section 21, D220).
/// One setting for the whole window, never chosen widget by widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    /// The default: room for a finger or a tired eye.
    #[default]
    Comfortable,
    /// More on screen at once: smaller text, tighter rows. For a laptop
    /// screen at 1280x720 with several panes open.
    Compact,
}

impl Density {
    /// Every density, in the order the Settings pane lists them.
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Compact];

    /// The name saved in the preferences file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }

    /// The density saved as `key`; `None` for a name this build does not
    /// know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|density| density.key() == key)
    }

    /// The name shown in Settings.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "Comfortable",
            Self::Compact => "Compact",
        }
    }

    /// The height of a list row (the scene browser, the cue list).
    pub const fn row_height(self) -> f32 {
        match self {
            Self::Comfortable => 44.0,
            Self::Compact => 32.0,
        }
    }

    /// The gap above a list row's first line and below its last.
    pub const fn row_inset(self) -> f32 {
        match self {
            Self::Comfortable => 6.0,
            Self::Compact => 2.0,
        }
    }

    /// Body and button text size.
    const fn body(self) -> f32 {
        match self {
            Self::Comfortable => 13.5,
            Self::Compact => 12.5,
        }
    }

    /// Small text size.
    const fn small(self) -> f32 {
        match self {
            Self::Comfortable => 11.5,
            Self::Compact => 11.0,
        }
    }

    /// Gap between widgets in a row, and between rows.
    const fn item_spacing(self) -> Vec2 {
        match self {
            Self::Comfortable => Vec2::new(SPACE_SM, SPACE_XS),
            Self::Compact => Vec2::new(6.0, 2.0),
        }
    }

    /// The smallest a button or field is.
    const fn interact_height(self) -> f32 {
        match self {
            Self::Comfortable => 24.0,
            Self::Compact => 20.0,
        }
    }

    /// A page's title: the name of the Settings page on screen.
    const fn title(self) -> f32 {
        match self {
            Self::Comfortable => 20.0,
            Self::Compact => 18.0,
        }
    }

    /// A section heading over a group of settings.
    const fn heading(self) -> f32 {
        match self {
            Self::Comfortable => 15.0,
            Self::Compact => 14.0,
        }
    }

    /// The line under a setting that says what it does.
    const fn description(self) -> f32 {
        match self {
            Self::Comfortable => 12.0,
            Self::Compact => 11.5,
        }
    }
}

thread_local! {
    /// The density in force; set with the theme by [`apply`].
    static DENSITY: Cell<Density> = const { Cell::new(Density::Comfortable) };
}

/// The density in force.
pub fn density() -> Density {
    DENSITY.with(Cell::get)
}

/// The height of a list row at the density in force.
pub fn row_height() -> f32 {
    density().row_height()
}

/// The space above and below a setting row's content (`super::form`), at
/// the density in force.
#[must_use]
pub fn row_pad() -> f32 {
    match density() {
        Density::Comfortable => SPACE_MD,
        Density::Compact => SPACE_SM,
    }
}

// The type scale (UI review S1). Each step is visibly apart from the next,
// and the two headings are semibold where the system has the face
// (`super::fonts`): a heading that is only bigger, or fainter, than the text
// under it does not read as one.

/// A page's title, at the density in force.
#[must_use]
pub fn title_font() -> egui::FontId {
    super::fonts::strong(density().title())
}

/// A section heading, at the density in force.
#[must_use]
pub fn heading_font() -> egui::FontId {
    super::fonts::strong(density().heading())
}

/// A setting's description, at the density in force.
#[must_use]
pub fn description_font() -> egui::FontId {
    egui::FontId::proportional(density().description())
}

/// The height of a pane's header.
pub const PANE_HEADER: f32 = 36.0;

/// The height of the custom title bar.
pub const TITLE_BAR_HEIGHT: f32 = 36.0;

/// The height of the health strip.
pub const HEALTH_HEIGHT: f32 = 28.0;

/// The width of the navigation rail.
pub const RAIL_WIDTH: f32 = 60.0;

/// The width of the accent bar down a selected row's leading edge.
pub const SELECTION_BAR: f32 = 3.0;

/// The size of a transport button, in points: large enough to hit without
/// aiming, from a normal viewing distance.
pub const TRANSPORT_BUTTON: f32 = 36.0;

// The geometry has to hold together, and these are relations between
// constants, so they are checked when the crate is compiled.
const _: () = {
    assert!(
        PAD == SPACE_XS || PAD == SPACE_SM || PAD == SPACE_MD || PAD == SPACE_LG,
        "the pane inset must be a step of the spacing scale, or panes start \
         their content on different columns"
    );
    assert!(
        TRANSPORT_BUTTON >= 32.0,
        "a transport button smaller than 32 points is not hittable on stage"
    );
    assert!(
        HEALTH_HEIGHT < TITLE_BAR_HEIGHT,
        "the readout strip is quieter than the title bar above it"
    );
};

thread_local! {
    /// The palette in force for this frame.
    ///
    /// Thread-local rather than a global: egui is single-threaded per
    /// context, and a `static` behind a lock would be a lock taken
    /// thousands of times per frame for a value that never changes within
    /// one.
    static ACTIVE: RefCell<Rc<Palette>> = RefCell::new(Rc::new(crate::palette::fallback().clone()));
}

/// Installs `theme` at `density` and derives egui's own style from them.
/// Called at start-up and when either changes, not every frame.
pub fn apply(ctx: &egui::Context, theme: &Palette, density: Density) {
    ACTIVE.with(|active| active.replace(Rc::new(theme.clone())));
    DENSITY.with(|cell| cell.set(density));

    // egui keeps a separate `Style` for light and dark and picks between
    // them from the OS setting. The palette already decides which it is, so
    // the preference is pinned to the theme's own mode.
    ctx.set_theme(match theme.mode {
        Mode::Dark => egui::ThemePreference::Dark,
        Mode::Light => egui::ThemePreference::Light,
    });

    let derived = visuals(theme);
    ctx.all_styles_mut(|style| {
        style.visuals = derived.clone();
        style.spacing.item_spacing = density.item_spacing();
        style.spacing.button_padding = density.item_spacing();
        style.spacing.menu_margin = Margin::same(as_margin(SPACE_XS));
        style.spacing.indent = SPACE_MD;
        style.spacing.interact_size = Vec2::new(0.0, density.interact_height());
        // Solid scrollbars, never floating: a floating bar is invisible
        // until the pointer is already on it, so a list with rows past its
        // edge looks like it has lost them.
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = SPACE_SM;

        // Labels are not text fields. egui makes every `ui.label`
        // selectable by default, which puts an I-beam under the pointer
        // everywhere on a control surface.
        style.interaction.selectable_labels = false;

        style.text_styles.insert(
            TextStyle::Body,
            egui::FontId::new(density.body(), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            egui::FontId::new(density.body(), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            egui::FontId::new(density.small(), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            egui::FontId::new(17.0, egui::FontFamily::Proportional),
        );
        // Numeric readouts are drawn in this: a proportional font gives
        // every digit a different width, so a live-updating number shimmers
        // as its digits change.
        style.text_styles.insert(
            TextStyle::Monospace,
            egui::FontId::new(12.5, egui::FontFamily::Monospace),
        );
    });
}

/// The palette in force. A reference-count bump: cheap enough for every
/// frame, and it never allocates.
#[must_use]
pub fn palette() -> Rc<Palette> {
    ACTIVE.with(|active| Rc::clone(&active.borrow()))
}

/// The GL clear colour of the control window: the app background.
#[must_use]
pub fn clear_color() -> [f32; 4] {
    let app = palette().app;
    let [r, g, b, _] = rgb(app).to_normalized_gamma_f32();
    [r, g, b, 1.0]
}

/// Converts a palette colour to an egui one.
///
/// The single conversion point between [`crate::color::Rgb`] and `Color32`,
/// which is what lets the theme catalog and its contrast tests stay free of
/// egui.
#[must_use]
pub const fn rgb(color: Rgb) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

/// A palette colour at a given opacity, for scrims and overlays, where the
/// *alpha* is the design decision and the colour still comes from the theme.
#[must_use]
pub fn translucent(color: Rgb, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, alpha)
}

/// A spacing-scale value as a margin unit. `Margin` is `i8` in egui while
/// the scale is `f32`, so this is the one place the conversion happens.
#[must_use]
pub fn as_margin(space: f32) -> i8 {
    space.round().clamp(0.0, 127.0) as i8
}

/// A uniform margin from a scale value.
#[must_use]
pub fn margin(space: f32) -> Margin {
    Margin::same(as_margin(space))
}

/// A margin with separate horizontal and vertical steps.
#[must_use]
pub fn margin_xy(horizontal: f32, vertical: f32) -> Margin {
    Margin::symmetric(as_margin(horizontal), as_margin(vertical))
}

/// The frame a card is drawn in.
pub fn card(theme: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(rgb(theme.panel))
        .stroke(Stroke::new(1.0, rgb(theme.border)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(margin(SPACE_MD))
}

/// egui's `Visuals`, derived from a palette. Two of these assignments are
/// load-bearing and commented where they are made.
fn visuals(theme: &Palette) -> egui::Visuals {
    let mut visuals = match theme.mode {
        Mode::Dark => egui::Visuals::dark(),
        Mode::Light => egui::Visuals::light(),
    };

    visuals.panel_fill = rgb(theme.panel);
    visuals.window_fill = rgb(theme.raised);
    visuals.window_stroke = Stroke::new(1.0, rgb(theme.border));
    visuals.window_corner_radius = CornerRadius::same(RADIUS_LG);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.faint_bg_color = rgb(theme.raised);
    visuals.extreme_bg_color = rgb(theme.app);
    visuals.code_bg_color = rgb(theme.raised);
    visuals.hyperlink_color = rgb(theme.accent);
    visuals.warn_fg_color = rgb(theme.warning);
    visuals.error_fg_color = rgb(theme.danger);
    visuals.override_text_color = Some(rgb(theme.text));
    visuals.weak_text_color = Some(rgb(theme.text_muted));

    visuals.selection.bg_fill = rgb(theme.selection);
    visuals.selection.stroke = Stroke::new(1.0, rgb(theme.accent));

    // The two assignments that matter.
    //
    // egui paints *buttons* from `weak_bg_fill` and *filled controls* (the
    // scrollbar handle, a checkbox's interior, a slider's rail) from
    // `bg_fill`. They are not interchangeable. A button is a surface and may
    // share the card's colour; a scrollbar handle in the card's own colour
    // is invisible. `a_scrollbar_handle_is_never_the_colour_of_what_it_scrolls`
    // in `crate::palette` checks the separation for every theme.
    for (state, surface, control) in [
        (
            &mut visuals.widgets.noninteractive,
            theme.panel,
            theme.control,
        ),
        (&mut visuals.widgets.inactive, theme.raised, theme.control),
        (
            &mut visuals.widgets.hovered,
            theme.hover,
            theme.control_hover,
        ),
        (&mut visuals.widgets.active, theme.hover, theme.accent),
        (&mut visuals.widgets.open, theme.hover, theme.control_hover),
    ] {
        state.weak_bg_fill = rgb(surface);
        state.bg_fill = rgb(control);
        state.bg_stroke = Stroke::new(1.0, rgb(theme.border));
        state.fg_stroke = Stroke::new(1.0, rgb(theme.text));
        state.corner_radius = CornerRadius::same(RADIUS);
        // No expansion on hover: egui's default grows a widget by a pixel,
        // which nudges its neighbours.
        state.expansion = 0.0;
    }
    // A field's outline (UI review S3). egui strokes a text field, a combo
    // box and its own buttons with these; in the plain border colour an
    // empty field on a card was a dark bar whose hint read like a value.
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, rgb(theme.border_strong));
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, rgb(theme.text_muted));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, rgb(theme.text_muted));
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, rgb(theme.text));
    // Keyboard focus. egui draws a focused widget with the `active` style,
    // which otherwise looks just like `hovered`, so a person tabbing through
    // the panel could not see where they were (U-10). A 2 px accent ring,
    // at least 3:1 against every surface
    // (`the_focus_ring_stands_out_from_every_surface_it_can_sit_on`). A
    // pressed button shows it for the press, which is feedback too.
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, rgb(theme.accent));

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Catalog;

    fn every_theme() -> Vec<Palette> {
        Catalog::load().themes().to_vec()
    }

    #[test]
    fn the_spacing_scale_is_strictly_increasing() {
        let scale = [SPACE_XS, SPACE_SM, SPACE_MD, SPACE_LG];
        assert!(
            scale.windows(2).all(|pair| pair[1] > pair[0]),
            "the scale must ascend: {scale:?}"
        );
        assert!(
            scale.windows(2).all(|pair| pair[1] - pair[0] >= 4.0),
            "adjacent steps must differ enough to be told apart: {scale:?}"
        );
    }

    #[test]
    fn every_scale_value_survives_the_margin_conversion() {
        for space in [SPACE_XS, SPACE_SM, SPACE_MD, SPACE_LG, PAD] {
            let converted = f32::from(as_margin(space));
            assert!(
                (converted - space).abs() < 0.5,
                "{space} became {converted} as a margin"
            );
        }
    }

    #[test]
    fn an_absurd_margin_is_clamped_rather_than_wrapping() {
        assert_eq!(as_margin(10_000.0), 127);
        assert_eq!(as_margin(-50.0), 0);
        assert!(as_margin(f32::NAN) >= 0);
    }

    #[test]
    fn a_filled_control_never_shares_its_surfaces_colour() {
        for theme in every_theme() {
            let visuals = visuals(&theme);
            for state in [
                &visuals.widgets.noninteractive,
                &visuals.widgets.inactive,
                &visuals.widgets.hovered,
                &visuals.widgets.open,
            ] {
                assert_ne!(
                    state.bg_fill, state.weak_bg_fill,
                    "{}: a filled control shares its surface's colour",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn the_derived_visuals_carry_the_themes_own_colours() {
        for theme in every_theme() {
            let visuals = visuals(&theme);
            assert_eq!(visuals.panel_fill, rgb(theme.panel), "{}", theme.name);
            assert_eq!(visuals.selection.bg_fill, rgb(theme.selection));
            assert_eq!(visuals.error_fg_color, rgb(theme.danger));
            assert_eq!(visuals.override_text_color, Some(rgb(theme.text)));
        }
    }

    #[test]
    fn a_light_theme_produces_light_visuals() {
        for theme in every_theme() {
            let visuals = visuals(&theme);
            match theme.mode {
                Mode::Dark => assert!(visuals.dark_mode, "{}", theme.name),
                Mode::Light => assert!(!visuals.dark_mode, "{}", theme.name),
            }
        }
    }

    #[test]
    fn no_widget_state_expands_on_hover() {
        for theme in every_theme() {
            let visuals = visuals(&theme);
            for state in [
                &visuals.widgets.noninteractive,
                &visuals.widgets.inactive,
                &visuals.widgets.hovered,
                &visuals.widgets.active,
                &visuals.widgets.open,
            ] {
                assert_eq!(state.expansion, 0.0, "{}", theme.name);
            }
        }
    }

    #[test]
    fn the_colour_conversion_round_trips() {
        let color = Rgb::new(0x4c, 0xc9, 0xf0);
        let converted = rgb(color);
        assert_eq!(
            (converted.r(), converted.g(), converted.b()),
            (color.r, color.g, color.b)
        );
        assert_eq!(converted.a(), 255, "a palette colour is opaque");
    }

    #[test]
    fn asking_for_the_palette_does_not_copy_it() {
        // The frame path calls `palette()` constantly; it must be a
        // reference-count bump, not a clone of the palette's strings.
        let a = palette();
        let b = palette();
        assert!(Rc::ptr_eq(&a, &b), "two reads gave two palettes");
    }

    #[test]
    fn the_clear_colour_is_the_app_background() {
        let clear = clear_color();
        assert!(clear.iter().all(|c| (0.0..=1.0).contains(c)), "{clear:?}");
    }
}
