// ============================================================================
// Module:       strobe_app::ui::perform
// Description:  The large live preview (the PLAY main view) with its
//               toolbar and state overlays, the banners for failures and
//               notices, and the fitted preview the side panel reuses.
//
// Dependencies: egui; super::{model, theme, widgets, settings, icon},
//               crate::{icon, workspace}
// ============================================================================

//! The live preview (UI_DESIGN section 7).
//!
//! The side panel shows the preview at its width in every view (D283);
//! PLAY, the main view with no pane open, shows it at full size with the
//! output controls above it. Both are the scene's own output texture
//! painted directly by egui (no readback, UI_DESIGN section 31), so they
//! work with the output window hidden.
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
//! | notice | a banner (a recovered context, a request that is not wired yet), with a button when it offers a way on (a refused write offers Copy to my library, D271) |

use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

/// The toolbar above the large preview, with the output controls
/// right-aligned only while the side panel is closed. One set of them on
/// screen at a time: the side panel's status strip has the same two, and
/// the output monitor is chosen in Settings, so with the panel open the
/// toolbar is just its label (UI_REVIEW, Play).
pub fn toolbar(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    side_shown: bool,
) {
    // The buttons' words need about this much; below it they are icons
    // with the words in their tooltips, rather than drawn over each other
    // (U-09, T480 Task 35).
    const LABELLED: f32 = 470.0;
    let labelled = ui.available_width() >= LABELLED;
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.label(
            egui::RichText::new("LIVE PREVIEW")
                .size(10.5)
                .strong()
                .color(theme::rgb(theme.text_muted)),
        )
        .on_hover_text(
            "The scene as the output shows it. Pick a view in the rail to put a pane here.",
        );
        if side_shown {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fullscreen = model.fullscreen && model.output_visible;
            if widgets::tool_button_fitted(
                ui,
                theme,
                Icon::Fullscreen,
                "Fullscreen",
                fullscreen,
                "Fullscreen on the chosen monitor; shows the output if it is hidden (F11)",
                labelled,
            )
            .clicked()
            {
                actions.push(Action::ToggleFullscreen);
            }
            let visible = model.output_visible;
            if widgets::tool_button_fitted(
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
                labelled,
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
            if widgets::button(ui, theme, None, "Open in Editor").clicked() {
                if let Some(at) = crate::editor::error_location(failure.summary) {
                    model.ui.editor.goto = Some((at.line, at.column));
                }
                actions.push(Action::OpenEditor(Some(failure.scene.to_owned())));
            }
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissFailure);
            }
            // Not while Diagnostics is the view: the button would go where
            // the operator already is.
            let in_diagnostics =
                model.ui.panes.shown_in(crate::workspace::Dock::Main) == Some(Pane::Diagnostics);
            if !in_diagnostics
                && widgets::button(ui, theme, Some(Icon::Diagnostics), "Diagnostics").clicked()
            {
                model.ui.panes.open(Pane::Diagnostics);
            }
        });
        ui.add_space(SPACE_XS);
    }
    if let Some(hint) = model.drop_hint {
        widgets::banner(ui, theme, Tone::Info, hint, |_| {});
        ui.add_space(SPACE_XS);
    }
    if let Some(notice) = model.notice {
        widgets::banner(ui, theme, notice.tone, notice.text, |ui| {
            // Right to left: Dismiss at the edge, the offer before it.
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissNotice);
            }
            if let Some(offer) = notice.offer
                && widgets::button(ui, theme, None, offer.label).clicked()
            {
                // Dismissed first, so a notice the offer itself raises
                // (the copy's result) stays up.
                actions.push(Action::DismissNotice);
                actions.push(offer.action.clone());
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

/// The live preview, fitted to the space left and centred. `offer` puts
/// a Show output button on the output-hidden chip.
pub fn preview(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    offer: bool,
) {
    preview_in(ui, model, theme, actions, false, offer);
}

/// [`preview`], `compact` for the side panel's small picture: the empty
/// and loading states are a line each, with no button, and the size
/// caption is left out.
///
/// `offer` puts a Show output button on the output-hidden chip. The
/// callers give it to one picture at a time, so a screen never has two
/// Show output buttons: PLAY's toolbar has one while the side panel is
/// closed; with it open, PLAY's large picture has it; in any other view,
/// the side panel's picture.
pub fn preview_in(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    compact: bool,
    offer: bool,
) {
    let area = ui.available_rect_before_wrap();
    // The surface is a click target only to take keyboard focus off
    // whatever had it, so the transport keys work again afterwards.
    // Two previews can share a frame (the side panel's and PLAY's), so
    // each has its own id.
    let surface = if compact {
        "side-preview-surface"
    } else {
        "preview-surface"
    };
    let response = ui.interact(area, egui::Id::unique(surface), Sense::click());
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

    let Some(image) = model.preview else {
        match model.loading {
            Some(name) => preparing(ui, theme, area, name, compact),
            None if compact => compact_status(ui, theme, area, Icon::Perform, "No scene playing"),
            None => no_scene(ui, model, theme, area),
        }
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
    // A scene being prepared: the live picture stays (the show is still
    // running behind it), dimmed under the explanation.
    if let Some(name) = model.loading {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, theme::translucent(theme.app, 190));
        preparing(ui, theme, rect, name, compact);
        ui.allocate_rect(area, Sense::hover());
        return;
    }

    let blackout = model.transport.blackout;
    if blackout {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(3.0, theme::rgb(theme.danger)),
            StrokeKind::Inside,
        );
    }
    let chip = badges(ui, model, theme, rect, actions, offer);
    if compact {
        ui.allocate_rect(area, Sense::hover());
        return;
    }
    // The render size in the bottom-right corner, unless a small preview
    // puts it under the output-hidden chip (U-09).
    let colour = theme::translucent(theme.text, 150);
    let caption = ui.painter().layout_no_wrap(
        format!("{}x{}", image.size[0], image.size[1]),
        FontId::new(10.5, egui::FontFamily::Monospace),
        colour,
    );
    let at = rect.right_bottom() - Vec2::new(SPACE_SM, SPACE_SM) - caption.size();
    if chip.is_none_or(|chip| !chip.intersects(Rect::from_min_size(at, caption.size()))) {
        ui.painter().galley(at, caption, colour);
    }
    ui.allocate_rect(area, Sense::hover());
}

/// The state badges over the picture's top-left corner and the output chip
/// over its bottom-left; returns the output chip's rectangle when it shows.
fn badges(
    ui: &mut Ui,
    model: &Model<'_>,
    theme: &Palette,
    rect: Rect,
    actions: &mut Vec<Action>,
    offer: bool,
) -> Option<Rect> {
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
    if model.output_visible {
        return None;
    }
    let chip = egui::Rect::from_min_size(
        rect.left_bottom() + Vec2::new(SPACE_SM, -SPACE_SM - 24.0),
        Vec2::new(rect.width() - SPACE_SM * 2.0, 24.0),
    );
    let used = ui.scope_builder(egui::UiBuilder::new().max_rect(chip), |ui| {
        ui.horizontal(|ui| {
            // Shorter in a small preview, and left to the button alone in
            // the smallest, so the row never runs past the picture (U-09).
            const FULL: f32 = 420.0;
            const SHORT: f32 = 240.0;
            let words = if rect.width() >= FULL {
                Some("Output hidden: this is the preview")
            } else if rect.width() >= SHORT {
                Some("Output hidden")
            } else {
                None
            };
            if let Some(words) = words {
                widgets::status_chip_over(ui, Icon::Output, words, theme.text, Some(theme.app));
            }
            if offer && widgets::button(ui, theme, None, "Show output").clicked() {
                actions.push(Action::SetOutputVisible(true));
            }
        });
    });
    Some(used.response.rect)
}

fn no_scene(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, area: Rect) {
    let (title, detail) = if model.library.entries.is_empty() {
        (
            "No scenes in the library",
            "Put scene folders in the library folder, or import a Shadertoy shader.",
        )
    } else if model.failure.is_some() {
        (
            "No scene playing",
            "The last scene failed to load. Pick another from the library.",
        )
    } else {
        (
            "No scene playing",
            "Click one in the library to make it live, or press the right arrow for the first.",
        )
    };
    centred_status(ui, theme, area, Icon::Perform, title, detail);
    let button_at = area.center() + Vec2::new(0.0, 56.0);
    let rect = Rect::from_center_size(button_at, Vec2::new(160.0, 30.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let empty = model.library.entries.is_empty();
        let label = if empty {
            "Import a shader"
        } else {
            "Open the library"
        };
        if widgets::primary_button(ui, theme, label).clicked() {
            model
                .ui
                .panes
                .open(if empty { Pane::Import } else { Pane::Browser });
        }
    });
}

fn preparing(ui: &mut Ui, theme: &Palette, area: Rect, name: &str, compact: bool) {
    if compact {
        compact_status(
            ui,
            theme,
            area,
            Icon::Refresh,
            &format!("Preparing {name}..."),
        );
        return;
    }
    centred_status(
        ui,
        theme,
        area,
        Icon::Refresh,
        &format!("Preparing {name}..."),
        "Compiling shaders. The current scene stays live until it is ready.",
    );
}

/// One line with an icon, centred: the small preview's empty and loading
/// states.
fn compact_status(ui: &mut Ui, theme: &Palette, area: Rect, icon: Icon, title: &str) {
    let galley = widgets::truncated(
        ui,
        title,
        egui::TextStyle::Small.resolve(ui.style()),
        theme.text_muted,
        (area.width() - icons::INLINE - SPACE_SM * 3.0).max(20.0),
    );
    let width = icons::INLINE + SPACE_XS + galley.size().x;
    let left = area.center().x - width / 2.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            egui::pos2(left + icons::INLINE / 2.0, area.center().y),
            Vec2::splat(icons::INLINE),
        ),
        icon,
        theme::rgb(theme.text_faint),
    );
    ui.painter().galley(
        egui::pos2(
            left + icons::INLINE + SPACE_XS,
            area.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::rgb(theme.text_muted),
    );
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
