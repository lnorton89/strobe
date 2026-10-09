// ============================================================================
// Module:       strobe_app::theme
// Description:  The app's look in one place: a dark egui style with a single
//               amber accent, and the few named colours drawing code uses.
//
// Dependencies: egui
// ============================================================================

//! The theme.
//!
//! Every colour in the UI comes from here. Drawing code asks for a named
//! role ([`accent`], [`error`], [`muted`]) and never writes a colour
//! literal, so changing the look, or replacing this module with the full
//! theme catalogue (D5), touches one file.
//!
//! The palette is deliberately small: near-black surfaces in four steps,
//! warm grey text, and amber as the only chromatic colour. Amber marks
//! what is live or selected; red is reserved for failures.

use egui::{Color32, Context, CornerRadius, Stroke, Theme, Visuals};

/// The window background, behind every panel.
const SURFACE_0: Color32 = Color32::from_rgb(14, 14, 16);
/// Panels.
const SURFACE_1: Color32 = Color32::from_rgb(22, 22, 25);
/// Widgets at rest.
const SURFACE_2: Color32 = Color32::from_rgb(34, 34, 38);
/// Widgets under the pointer.
const SURFACE_3: Color32 = Color32::from_rgb(48, 48, 54);
/// Body text.
const TEXT: Color32 = Color32::from_rgb(222, 220, 214);
/// Secondary text.
const TEXT_MUTED: Color32 = Color32::from_rgb(140, 138, 132);
/// Emphasised text: headings, and a pressed widget's label.
const TEXT_STRONG: Color32 = Color32::from_rgb(250, 248, 242);
/// The accent: amber.
const AMBER: Color32 = Color32::from_rgb(255, 176, 32);
/// Failures.
const ERROR: Color32 = Color32::from_rgb(240, 96, 84);
/// Warnings: paler than the accent so a warning never reads as "live".
const WARNING: Color32 = Color32::from_rgb(206, 168, 96);

/// Corner radius of widgets, in points.
const RADIUS: u8 = 3;

/// The accent colour, for what is live or selected.
pub fn accent() -> Color32 {
    AMBER
}

/// The colour of failure text.
pub fn error() -> Color32 {
    ERROR
}

/// The colour of warning text.
pub fn warning() -> Color32 {
    WARNING
}

/// The colour of secondary text.
pub fn muted() -> Color32 {
    TEXT_MUTED
}

/// The colour behind the preview image, so letterbox bars read as bars.
pub fn preview_backdrop() -> Color32 {
    SURFACE_0
}

/// The GL clear colour of the control window, as linear RGBA.
pub fn clear_color() -> [f32; 4] {
    let [r, g, b, _] = SURFACE_0.to_normalized_gamma_f32();
    [r, g, b, 1.0]
}

/// Installs the theme on `ctx`, and pins egui to dark so a light system
/// setting cannot swap it out.
pub fn apply(ctx: &Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals());
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(TEXT_MUTED);
    v.panel_fill = SURFACE_1;
    v.window_fill = SURFACE_1;
    v.extreme_bg_color = SURFACE_0;
    v.faint_bg_color = SURFACE_2;
    v.code_bg_color = SURFACE_0;
    v.hyperlink_color = AMBER;
    v.warn_fg_color = AMBER;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = AMBER.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, AMBER);
    v.slider_trailing_fill = true;

    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE_1;
    w.noninteractive.weak_bg_fill = SURFACE_1;
    w.noninteractive.bg_stroke = Stroke::new(1.0, SURFACE_2);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.bg_fill = SURFACE_2;
    w.inactive.weak_bg_fill = SURFACE_2;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = SURFACE_3;
    w.hovered.weak_bg_fill = SURFACE_3;
    w.hovered.bg_stroke = Stroke::new(1.0, AMBER.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.active.bg_fill = SURFACE_3;
    w.active.weak_bg_fill = SURFACE_3;
    w.active.bg_stroke = Stroke::new(1.0, AMBER);
    w.active.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    w.open.bg_fill = SURFACE_3;
    w.open.weak_bg_fill = SURFACE_3;
    w.open.bg_stroke = Stroke::new(1.0, AMBER.gamma_multiply(0.6));
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = radius;
    }
    v
}
