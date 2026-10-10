// ============================================================================
// Module:       strobe_app::ui::perform
// Description:  The Perform work surface: the toolbar above the preview, the
//               banners for failures and notices, and the live preview with
//               its state overlays.
//
// Dependencies: egui; super::{model, theme, widgets, settings, icon},
//               crate::{icon, workspace}
// ============================================================================

//! The Perform surface (UI_DESIGN section 7).
//!
//! The preview is the primary object on screen: it takes everything the
//! panes, the strip and the toolbar leave. It is the scene's own output
//! texture painted directly by egui (no readback, UI_DESIGN section 31), so
//! it works with the output window hidden.
//!
//! ## States
//!
//! The surface always says which of these it is in, in words and an icon:
//!
//! | State | What shows |
//! |---|---|
//! | no scene | "No scene playing" with a button that opens the library |
//! | loading | the name of the scene being prepared, with a spinner |
//! | playing | the picture, a `LIVE` badge |
//! | blackout | the picture, a danger border, a `BLACKOUT` badge: the output is dark but the operator can still see the scene |
//! | output hidden | a chip that says the picture is preview-only, with a button to show the output |
//! | scene failed | a banner above the picture; the old scene keeps playing |
//! | notice | a banner (a recovered context, a request that is not wired yet) |

use super::icon as icons;
use super::model::{Action, Model};
use super::settings;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::workspace::{Arrangement, Pane};
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

/// The toolbar above the preview: the panes on the left, the output on the
/// right.
pub fn toolbar(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    arrangement: &Arrangement,
    actions: &mut Vec<Action>,
) {
    let shown = |pane: Pane| {
        arrangement.left.is_some_and(|(p, _)| p == pane)
            || arrangement.right.is_some_and(|(p, _)| p == pane)
    };
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        if widgets::tool_button(
            ui,
            theme,
            Icon::Library,
            "Scenes",
            shown(Pane::Browser),
            "Browse, search and preview scenes (Ctrl+F)",
        )
        .clicked()
        {
            model.ui.panes.toggle(Pane::Browser);
        }
        if widgets::tool_button(
            ui,
            theme,
            Icon::Controls,
            "Controls",
            shown(Pane::Inspector),
            "The playing scene's controls",
        )
        .clicked()
        {
            model.ui.panes.toggle(Pane::Inspector);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let wide = ui.available_width() > 560.0;
            if wide {
                settings::monitor_picker(ui, model, actions);
            }
            let fullscreen = model.fullscreen && model.output_visible;
            if widgets::tool_button(
                ui,
                theme,
                Icon::Fullscreen,
                "Fullscreen",
                fullscreen,
                "Fullscreen on the chosen monitor; shows the output if it is hidden (F11)",
            )
            .clicked()
            {
                actions.push(Action::ToggleFullscreen);
            }
            let visible = model.output_visible;
            if widgets::tool_button(
                ui,
                theme,
                Icon::Output,
                if visible {
                    "Hide output"
                } else {
                    "Show output"
                },
                visible,
                "The output window starts hidden. The scene keeps rendering while it is.",
            )
            .clicked()
            {
                actions.push(Action::SetOutputVisible(!visible));
            }
        });
    });
}

/// The failure and notice banners, above the preview.
pub fn banners(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    if let Some(failure) = model.failure {
        let message = match failure.kept {
            Some(kept) => format!(
                "{} did not load: {}. {} keeps playing.",
                failure.scene, failure.summary, kept
            ),
            None => format!(
                "{} did not load: {}. Nothing is playing.",
                failure.scene, failure.summary
            ),
        };
        widgets::banner(ui, theme, Tone::Error, &message, |ui| {
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissFailure);
            }
            if widgets::button(ui, theme, Some(Icon::Diagnostics), "Diagnostics").clicked() {
                model.ui.panes.open(Pane::Diagnostics);
            }
        });
        ui.add_space(SPACE_XS);
    }
    if let Some(notice) = model.notice {
        widgets::banner(ui, theme, notice.tone, notice.text, |ui| {
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissNotice);
            }
        });
        ui.add_space(SPACE_XS);
    }
}

/// The largest rectangle of `aspect` (width over height) inside `bounds`.
pub fn fit_inside(bounds: Vec2, aspect: f32) -> Vec2 {
    if !aspect.is_finite()
        || aspect <= 0.0
        || !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || bounds.x <= 0.0
        || bounds.y <= 0.0
    {
        return Vec2::ZERO;
    }
    if bounds.x / bounds.y > aspect {
        Vec2::new(bounds.y * aspect, bounds.y)
    } else {
        Vec2::new(bounds.x, bounds.x / aspect)
    }
}

/// The live preview, fitted to the space left and centred.
pub fn preview(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let area = ui.available_rect_before_wrap();
    // The surface is a click target only to take keyboard focus off
    // whatever had it, so the transport keys work again afterwards.
    let response = ui.interact(area, egui::Id::new("preview-surface"), Sense::click());
    if response.clicked() {
        ui.ctx().memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    ui.painter().rect_filled(
        area,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.app),
    );

    if let Some(name) = model.loading {
        centred_status(
            ui,
            theme,
            area,
            Icon::Refresh,
            &format!("Preparing {name}..."),
            "Compiling shaders. The picture returns when it is ready.",
        );
        ui.allocate_rect(area, Sense::hover());
        return;
    }
    let Some(image) = model.preview else {
        no_scene(ui, model, theme, area);
        ui.allocate_rect(area, Sense::hover());
        return;
    };

    let fit = fit_inside(area.size() - Vec2::splat(SPACE_XS * 2.0), image.aspect);
    let rect = Rect::from_center_size(area.center(), fit);
    // The player's targets are bottom-up, egui's texture space is top-down.
    let uv = Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
    ui.painter().image(
        image.texture,
        rect,
        uv,
        egui::Color32::from_white_alpha(255),
    );

    let blackout = model.transport.blackout;
    if blackout {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(3.0, theme::rgb(theme.danger)),
            StrokeKind::Inside,
        );
    }
    badges(ui, model, theme, rect, actions);
    ui.painter().text(
        rect.right_bottom() - Vec2::new(SPACE_SM, SPACE_SM),
        Align2::RIGHT_BOTTOM,
        format!("{}x{}", image.size[0], image.size[1]),
        FontId::new(10.5, egui::FontFamily::Monospace),
        theme::translucent(theme.text, 150),
    );
    ui.allocate_rect(area, Sense::hover());
}

/// The state badges over the picture's top-left corner and the output chip
/// over its bottom-left.
fn badges(ui: &mut Ui, model: &Model<'_>, theme: &Palette, rect: Rect, actions: &mut Vec<Action>) {
    let badge = egui::Rect::from_min_size(
        rect.left_top() + Vec2::splat(SPACE_SM),
        Vec2::new(rect.width() - SPACE_SM * 2.0, 24.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(badge), |ui| {
        ui.horizontal(|ui| {
            if model.transport.blackout {
                widgets::status_chip_over(
                    ui,
                    Icon::Blackout,
                    "BLACKOUT: the output is dark",
                    theme.danger,
                    Some(theme.app),
                );
            } else {
                widgets::status_chip_over(ui, Icon::Play, "LIVE", theme.accent, Some(theme.app));
            }
        });
    });
    if !model.output_visible {
        let chip = egui::Rect::from_min_size(
            rect.left_bottom() + Vec2::new(SPACE_SM, -SPACE_SM - 24.0),
            Vec2::new(rect.width() - SPACE_SM * 2.0, 24.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(chip), |ui| {
            ui.horizontal(|ui| {
                widgets::status_chip_over(
                    ui,
                    Icon::Output,
                    "Output hidden: this is the preview",
                    theme.text,
                    Some(theme.app),
                );
                if widgets::button(ui, theme, None, "Show output").clicked() {
                    actions.push(Action::SetOutputVisible(true));
                }
            });
        });
    }
}

fn no_scene(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, area: Rect) {
    let (title, detail) = if model.library.entries.is_empty() {
        (
            "No scenes in the library",
            "Open the library to rescan, or import a Shadertoy shader.",
        )
    } else if model.failure.is_some() {
        (
            "No scene playing",
            "The last scene failed to load. Pick another from the library.",
        )
    } else {
        (
            "No scene playing",
            "Open the library and load one, or press the right arrow for the first.",
        )
    };
    centred_status(ui, theme, area, Icon::Perform, title, detail);
    let button_at = area.center() + Vec2::new(0.0, 56.0);
    let rect = Rect::from_center_size(button_at, Vec2::new(160.0, 30.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        if widgets::primary_button(ui, theme, "Open the library").clicked() {
            model.ui.panes.open(Pane::Browser);
        }
    });
}

fn centred_status(ui: &mut Ui, theme: &Palette, area: Rect, icon: Icon, title: &str, detail: &str) {
    let centre = area.center();
    let painter = ui.painter();
    icons::paint(
        painter,
        Rect::from_center_size(centre - Vec2::new(0.0, 46.0), Vec2::splat(34.0)),
        icon,
        theme::rgb(theme.text_faint),
    );
    painter.text(
        centre - Vec2::new(0.0, 14.0),
        Align2::CENTER_CENTER,
        title,
        FontId::new(17.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text),
    );
    let galley = widgets::truncated(
        ui,
        detail,
        egui::TextStyle::Body.resolve(ui.style()),
        theme.text_muted,
        area.width() - theme::SPACE_LG * 2.0,
    );
    ui.painter().galley(
        egui::pos2(centre.x - galley.size().x / 2.0, centre.y + 6.0),
        galley,
        theme::rgb(theme.text_muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_in_a_tall_panel_fills_the_width() {
        let fit = fit_inside(Vec2::new(400.0, 800.0), 16.0 / 9.0);
        assert!((fit.x - 400.0).abs() < 0.01, "width {}", fit.x);
        assert!((fit.y - 225.0).abs() < 0.01, "height {}", fit.y);
    }

    #[test]
    fn a_wide_frame_in_a_wide_panel_fills_the_height() {
        let fit = fit_inside(Vec2::new(1600.0, 450.0), 16.0 / 9.0);
        assert!((fit.y - 450.0).abs() < 0.01, "height {}", fit.y);
        assert!((fit.x - 800.0).abs() < 0.01, "width {}", fit.x);
    }

    #[test]
    fn nonsense_bounds_or_aspect_fit_to_nothing_rather_than_nan() {
        for fit in [
            fit_inside(Vec2::new(-5.0, 100.0), 1.5),
            fit_inside(Vec2::new(100.0, 100.0), 0.0),
            fit_inside(Vec2::new(100.0, 100.0), f32::NAN),
            fit_inside(Vec2::new(f32::NAN, 100.0), 1.5),
        ] {
            assert!(fit.x.is_finite() && fit.y.is_finite(), "{fit:?}");
        }
    }
}
