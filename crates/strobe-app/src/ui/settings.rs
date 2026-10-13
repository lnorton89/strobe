// ============================================================================
// Module:       strobe_app::ui::settings
// Description:  The settings pane: output window, performance, audio,
//               appearance, and the keyboard reference.
//
// Dependencies: egui; super::{model, theme, widgets, audio}, crate::{commands,
//               session}
// ============================================================================

//! Settings: everything the operator sets once and rarely touches, kept out
//! of Perform (UI_DESIGN section 3: system health and configuration come
//! after the output and the immediate controls).
//!
//! Sections, in the order a soundcheck goes:
//!
//! - **Output**: show or hide the window, fullscreen, which monitor (D96).
//! - **Performance**: adaptive resolution and the render scale (D64).
//! - **Audio**: the input device, reactivity, speed and tap tempo.
//! - **Appearance**: the theme, reduced motion, and the title bar (D132).
//! - **Keyboard**: the command table, as data, so what is advertised here
//!   is what exists.
//!
//! The render-scale slider edits a working value and only commits when the
//! drag ends, because every change reallocates every render target.

use super::audio;
use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::commands::CommandId;
use crate::icon::Icon;
use crate::palette::Palette;
use crate::session::RENDER_SCALE;
use crate::workspace::Pane;
use egui::Ui;

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(ui, theme, Icon::Settings, Pane::Settings.title(), |_| {});
    if closed {
        model.ui.panes.close(Pane::Settings);
    }
    egui::ScrollArea::vertical()
        .id_salt("settings-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            widgets::section(ui, theme, "Output");
            output(ui, model, theme, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Performance");
            performance(ui, model, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Audio");
            audio::show(ui, theme, &mut model.audio, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Appearance");
            appearance(ui, model, theme, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Keyboard");
            keyboard(ui, theme, model.remote.bindings);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Graphics");
            ui.label(
                egui::RichText::new(model.gl_summary)
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
        });
}

fn output(ui: &mut Ui, model: &Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        let visible = model.output_visible;
        let (icon, label) = if visible {
            (Icon::Output, "Hide output")
        } else {
            (Icon::Output, "Show output")
        };
        if widgets::button(ui, theme, Some(icon), label)
            .on_hover_text(
                "The output window starts hidden. The scene keeps rendering while it is, \
                 and the preview shows it.",
            )
            .clicked()
        {
            actions.push(Action::SetOutputVisible(!visible));
        }
        let fullscreen = if model.fullscreen && visible {
            "Exit fullscreen (F11)"
        } else {
            "Fullscreen (F11)"
        };
        if widgets::button(ui, theme, Some(Icon::Fullscreen), fullscreen)
            .on_hover_text("Fullscreen on the chosen monitor; shows the output if it is hidden.")
            .clicked()
        {
            actions.push(Action::ToggleFullscreen);
        }
    });
    ui.add_space(SPACE_XS);
    monitor_picker(ui, model, actions);
}

/// The output-monitor combo box.
pub fn monitor_picker(ui: &mut Ui, model: &Model<'_>, actions: &mut Vec<Action>) {
    let current = model
        .output_monitor
        .and_then(|index| model.monitors.get(index))
        .map_or("Output monitor", String::as_str);
    egui::ComboBox::from_id_salt("output-monitor")
        .selected_text(current)
        .show_ui(ui, |ui| {
            if model.monitors.is_empty() {
                ui.label("No monitors reported");
            }
            for (index, label) in model.monitors.iter().enumerate() {
                if ui
                    .selectable_label(model.output_monitor == Some(index), label)
                    .clicked()
                {
                    actions.push(Action::MoveToMonitor(index));
                }
            }
        });
}

fn performance(ui: &mut Ui, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    let mut adaptive = model.perf.adaptive;
    let toggle = ui
        .add_enabled(
            model.perf.has_timer,
            egui::Checkbox::new(&mut adaptive, "Adaptive resolution"),
        )
        .on_hover_text(
            "Lower the render scale when the GPU misses its frame budget, and raise it \
             again when there is headroom. The slider sets the maximum.",
        )
        .on_disabled_hover_text("This graphics context cannot time the GPU");
    if toggle.changed() {
        actions.push(Action::SetAdaptive(adaptive));
    }
    let label = if model.perf.adaptive && model.perf.has_timer {
        "Maximum render scale"
    } else {
        "Render scale"
    };
    ui.label(
        egui::RichText::new(label)
            .small()
            .color(theme::rgb(theme::palette().text_muted)),
    );
    ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(60.0);
    let slider = ui.add(
        egui::Slider::new(model.render_scale, RENDER_SCALE)
            .step_by(0.05)
            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
    );
    if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
        actions.push(Action::SetRenderScale(*model.render_scale));
    }
}

fn appearance(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let current = model.ui.palette().name.clone();
    let mut chosen = None;
    egui::ComboBox::from_id_salt("theme")
        .selected_text(current)
        .show_ui(ui, |ui| {
            for palette in model.ui.catalog().themes() {
                let label = format!(
                    "{}{}",
                    palette.name,
                    match palette.mode {
                        crate::palette::Mode::Dark => "",
                        crate::palette::Mode::Light => " (light)",
                    }
                );
                if ui
                    .selectable_label(palette.id == model.ui.theme_id(), label)
                    .clicked()
                {
                    chosen = Some(palette.id.clone());
                }
            }
        });
    if let Some(id) = chosen {
        model.ui.set_theme(&id);
    }
    for (source, reason) in model.ui.catalog().problems() {
        ui.label(
            egui::RichText::new(format!("Theme file {source} not used: {reason}"))
                .small()
                .color(theme::rgb(theme.warning)),
        );
    }
    let mut reduced = model.ui.reduced_motion();
    if ui
        .checkbox(&mut reduced, "Reduce motion")
        .on_hover_text("No transitions and no overshoot. STROBE_REDUCED_MOTION=1 forces it on.")
        .changed()
    {
        model.ui.set_reduced_motion(reduced);
    }
    let mut custom = model.ui.custom_title_bar;
    if ui
        .checkbox(&mut custom, "Strobe title bar")
        .on_hover_text(
            "Draw Strobe's own title bar with the health readouts in it. Turn it off to use \
             the system's, if dragging or snapping misbehaves on this machine.",
        )
        .changed()
    {
        actions.push(Action::SetCustomTitleBar(custom));
    }
}

fn keyboard(ui: &mut Ui, theme: &Palette, bindings: &crate::commands::Bindings) {
    for command in CommandId::ALL {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(
                    bindings
                        .chord(command)
                        .map_or_else(|| "none".to_owned(), crate::commands::Chord::label),
                )
                .monospace()
                .color(theme::rgb(theme.accent)),
            );
            ui.label(
                egui::RichText::new(command.name())
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
        });
    }
    ui.label(
        egui::RichText::new(
            "Shortcuts are off while a text field has focus. Change one under \"hotkeys\" in \
             show-control.json (for example \"toggle_blackout\": \"Ctrl+B\"); restart to apply.",
        )
        .small()
        .color(theme::rgb(theme.text_faint)),
    );
}
