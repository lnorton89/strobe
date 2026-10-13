// ============================================================================
// Module:       strobe_app::ui::settings
// Description:  The settings pane: output window, performance, audio,
//               appearance, the watched folder, the LAN remote, and the
//               keyboard reference.
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
//! - **Watched folder**: a folder whose new scene folders join the library,
//!   playing none (D240). The switch and Apply act at once; the app checks
//!   the folder as at launch, and saves it only when it is accepted (D265).
//! - **Remote**: the LAN remote's switch, address, pairing and paired
//!   devices (`super::lan`, D254).
//! - **Layouts**: the open panes, their order and widths, saved under a
//!   name and put back in one click (D215).
//! - **Keyboard**: the command table, as data, so what is advertised here
//!   is what exists, and the shortcut editor (D208): Change waits for a key,
//!   a key another command holds is named and moved only on a second click,
//!   and Reset returns the default. Changes apply at once and are saved
//!   under `hotkeys` in `show-control.json`.
//!
//! The render-scale slider edits a working value and only commits when the
//! drag ends, because every change reallocates every render target.

use super::audio;
use super::keys::{self, HotkeyEditor};
use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::commands::{Bindings, Chord, CommandId};
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
            widgets::section(ui, theme, "Watched folder");
            watch_folder(ui, theme, &mut model.ui.watch, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Remote");
            super::lan::show(ui, theme, &mut model.lan, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Layouts");
            layouts(ui, theme, model.ui);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Keyboard");
            keyboard(
                ui,
                theme,
                model.remote.bindings,
                &mut model.ui.hotkeys,
                actions,
            );
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
    ui.horizontal(|ui| {
        ui.label("Density");
        let current = model.ui.density();
        for density in theme::Density::ALL {
            if ui
                .selectable_label(current == density, density.label())
                .on_hover_text(match density {
                    theme::Density::Comfortable => "Room to aim: the default",
                    theme::Density::Compact => {
                        "Smaller text and tighter rows, for a small screen with several panes open"
                    }
                })
                .clicked()
            {
                model.ui.set_density(density);
            }
        }
    });
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

/// Named layouts: save the panes as they are, apply or delete one (D215).
/// The watched drop folder (D265): a path, Apply, and a switch, with the
/// app's line saying what is watched or why not. Nothing here touches the
/// disk: the app checks the folder when the action arrives.
fn watch_folder(
    ui: &mut Ui,
    theme: &Palette,
    watch: &mut super::state::WatchState,
    actions: &mut Vec<Action>,
) {
    let mut on = watch.watching;
    if ui
        .checkbox(&mut on, "Watch a folder for new scenes")
        .on_hover_text(
            "A scene folder, or a folder of scenes, copied into it is added to the library, \
             a scene a frame, and nothing plays. What is in it when watching starts is left \
             alone.",
        )
        .changed()
    {
        actions.push(Action::SetWatchFolder {
            folder: watch.draft.trim().to_string(),
            enabled: on,
        });
    }
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut watch.draft)
                .hint_text("a folder beside the library")
                .desired_width((ui.available_width() - WATCH_BUTTON).max(WATCH_FIELD)),
        );
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let ready = !watch.draft.trim().is_empty();
        let apply = ui
            .add_enabled_ui(ready, |ui| {
                widgets::button(ui, theme, None, "Apply")
                    .on_hover_text("Watch this folder now, and from the next launch")
            })
            .inner;
        if ready && (apply.clicked() || entered) {
            actions.push(Action::SetWatchFolder {
                folder: watch.draft.trim().to_string(),
                enabled: true,
            });
        }
    });
    ui.label(
        egui::RichText::new(&watch.note)
            .small()
            .color(theme::rgb(if watch.failed {
                theme.danger
            } else {
                theme.text_muted
            })),
    );
}

/// The room the watched folder's field leaves for Apply.
const WATCH_BUTTON: f32 = 80.0;

/// The narrowest the watched folder's field gets.
const WATCH_FIELD: f32 = 80.0;

fn layouts(ui: &mut Ui, theme: &Palette, state: &mut super::state::UiState) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.layout_name)
                .hint_text("Prep, Show, Debug...")
                .desired_width((ui.available_width() - 150.0).max(80.0)),
        );
        let name = state.layout_name.trim().to_string();
        let replacing = state.layouts.contains_key(&name);
        let label = if replacing { "Replace" } else { "Save current" };
        let save = ui
            .add_enabled_ui(!name.is_empty(), |ui| {
                widgets::button(ui, theme, None, label).on_hover_text(if replacing {
                    "Replace this layout with the panes as they are now"
                } else {
                    "Save the open panes, their order and widths under this name"
                })
            })
            .inner;
        if save.clicked() && state.save_layout(&name) {
            state.layout_name.clear();
        }
    });
    if state.layouts.is_empty() {
        ui.label(
            egui::RichText::new("No saved layouts.")
                .small()
                .color(theme::rgb(theme.text_faint)),
        );
        return;
    }
    let names: Vec<String> = state.layouts.keys().cloned().collect();
    for name in names {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).color(theme::rgb(theme.text)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    state.delete_layout(&name);
                }
                if ui
                    .small_button("Apply")
                    .on_hover_text("Open these panes, in this order, at these widths")
                    .clicked()
                {
                    state.apply_layout(&name);
                }
            });
        });
    }
}

fn keyboard(
    ui: &mut Ui,
    theme: &Palette,
    bindings: &Bindings,
    editor: &mut HotkeyEditor,
    actions: &mut Vec<Action>,
) {
    if let Some(command) = editor.capturing {
        match keys::captured(ui.ctx()) {
            keys::Capture::Chord(chord) => {
                editor.capturing = None;
                actions.push(Action::SetHotkey {
                    command,
                    chord: Some(chord),
                    steal: false,
                });
            }
            keys::Capture::Waiting => {}
            keys::Capture::Cancel => editor.capturing = None,
        }
    }
    for command in CommandId::ALL {
        ui.horizontal(|ui| {
            let capturing = editor.capturing == Some(command);
            let label = if capturing {
                "press a key".to_owned()
            } else {
                bindings
                    .chord(command)
                    .map_or_else(|| "none".to_owned(), Chord::label)
            };
            let custom = bindings.chord(command) != command.chord();
            let colour = if capturing {
                theme.warning
            } else if custom {
                theme.info
            } else {
                theme.accent
            };
            ui.add_sized(
                [110.0, 18.0],
                egui::Label::new(
                    egui::RichText::new(label)
                        .monospace()
                        .color(theme::rgb(colour)),
                ),
            );
            ui.label(
                egui::RichText::new(command.name())
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if capturing {
                    if ui.small_button("Cancel").clicked() {
                        editor.capturing = None;
                    }
                    return;
                }
                if custom
                    && ui
                        .small_button("Reset")
                        .on_hover_text(command.chord().map_or_else(
                            || "Back to no shortcut".to_owned(),
                            |c| format!("Back to {}", c.label()),
                        ))
                        .clicked()
                {
                    actions.push(Action::ResetHotkey(command));
                }
                if bindings.chord(command).is_some()
                    && ui
                        .small_button("Clear")
                        .on_hover_text("No shortcut for this command")
                        .clicked()
                {
                    actions.push(Action::SetHotkey {
                        command,
                        chord: None,
                        steal: false,
                    });
                }
                if ui
                    .small_button("Change")
                    .on_hover_text("Then press the new key; Esc cancels")
                    .clicked()
                {
                    editor.capturing = Some(command);
                    editor.clash = None;
                }
            });
        });
        if let Some((wanted, chord, holder)) = editor.clash
            && wanted == command
        {
            let mut done = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} is {}.", chord.label(), holder.name()))
                        .small()
                        .color(theme::rgb(theme.warning)),
                );
                if ui
                    .small_button("Move it here")
                    .on_hover_text(format!("{} will have no shortcut", holder.name()))
                    .clicked()
                {
                    actions.push(Action::SetHotkey {
                        command,
                        chord: Some(chord),
                        steal: true,
                    });
                    done = true;
                }
                if ui.small_button("Keep it there").clicked() {
                    done = true;
                }
            });
            if done {
                editor.clash = None;
            }
        }
    }
    ui.add_space(SPACE_XS);
    ui.label(
        egui::RichText::new(
            "Shortcuts are off while a text field has focus. Changes apply at once and are \
             saved to show-control.json.",
        )
        .small()
        .color(theme::rgb(theme.text_faint)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use egui::{Pos2, Vec2};

    /// One pass of the watched-folder section; returns where each text was
    /// drawn.
    fn draw(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        watch: &mut super::super::state::WatchState,
        actions: &mut Vec<Action>,
    ) -> Vec<(String, egui::Rect)> {
        let theme = crate::palette::fallback().clone();
        let pass = crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(480.0, 160.0), events),
            |ui| watch_folder(ui, &theme, watch, actions),
        );
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

    fn click(
        ctx: &egui::Context,
        at: Pos2,
        watch: &mut super::super::state::WatchState,
        actions: &mut Vec<Action>,
    ) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = draw(
            ctx,
            vec![egui::Event::PointerMoved(at), button(true)],
            watch,
            actions,
        );
        let _ = draw(ctx, vec![button(false)], watch, actions);
    }

    fn find(drawn: &[(String, egui::Rect)], text: &str) -> Result<Pos2> {
        drawn
            .iter()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| rect.center())
            .ok_or_else(|| anyhow::anyhow!("no {text:?} in {drawn:?}"))
    }

    #[test]
    fn apply_and_the_switch_ask_the_app_and_change_nothing_themselves() -> Result<()> {
        let ctx = egui::Context::default();
        let mut watch = super::super::state::WatchState {
            draft: "  D:/VJ/inbox ".to_string(),
            note: "Off".to_string(),
            ..super::super::state::WatchState::default()
        };
        let mut actions = Vec::new();
        let drawn = draw(&ctx, Vec::new(), &mut watch, &mut actions);
        click(&ctx, find(&drawn, "Apply")?, &mut watch, &mut actions);
        assert_eq!(
            actions,
            vec![Action::SetWatchFolder {
                folder: "D:/VJ/inbox".to_string(),
                enabled: true,
            }],
            "Apply asks to watch the typed folder, trimmed"
        );
        assert!(
            !watch.watching && !watch.enabled && watch.folder.is_empty(),
            "the pane changes nothing itself: the app checks and saves: {watch:?}"
        );
        actions.clear();
        watch.watching = true;
        let drawn = draw(&ctx, Vec::new(), &mut watch, &mut actions);
        click(
            &ctx,
            find(&drawn, "Watch a folder for new scenes")?,
            &mut watch,
            &mut actions,
        );
        assert_eq!(
            actions,
            vec![Action::SetWatchFolder {
                folder: "D:/VJ/inbox".to_string(),
                enabled: false,
            }],
            "unticking a watch that runs asks to stop it"
        );
        Ok(())
    }
}
