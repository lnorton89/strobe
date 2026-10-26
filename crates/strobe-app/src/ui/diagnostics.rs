// ============================================================================
// Module:       strobe_app::ui::diagnostics
// Description:  The diagnostics pane: status cards for each subsystem, the
//               scenes that need work, and the bounded console.
//
// Dependencies: egui; super::{model, theme, widgets}, crate::{console, library}
// ============================================================================

//! Diagnostics (UI_IMPLEMENTATION_PLAN section 14, levels 2 and 3).
//!
//! The title bar's readouts are level 1: always there, never a dialog.
//! This pane is the investigation: one card per subsystem, each with a
//! status chip (icon and word, never colour alone) and the facts behind it;
//! the scenes that failed to load, with why; and the console.
//!
//! The console is not a second log. It is a view of `strobe-diag`'s ring
//! (D60), so it shows what the session file and a crash report contain,
//! including the structured `SHADER` / `AUDIO` / `IMPORT` failure blocks that
//! carry subsystem, subject, phase, root message and what the app did about
//! it. It is bounded (the last 200 records) and drawn only while this pane
//! is open.
//!
//! The pane is one page of cards that scrolls as a whole (D483): the
//! subsystem cards, two to a row where there is room; Needs work as a card
//! with a row per failed scene (its name, the first line of why, and Open
//! in Editor); and the console as a card of a fixed height that scrolls
//! inside, where it used to take whatever the cards left.

use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::console::Level;
use crate::icon::Icon;
use crate::library::Health;
use crate::palette::Palette;
use crate::script::ScriptState;
use crate::workspace::Pane;
use egui::{CornerRadius, Ui};

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(
        ui,
        theme,
        Icon::Diagnostics,
        Pane::Diagnostics.title(),
        |_| {},
    );
    if closed {
        model.ui.panes.close(Pane::Diagnostics);
    }
    egui::ScrollArea::vertical()
        .id_salt("diagnostics-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            cards(ui, model, theme);
            needs_work(ui, model, theme, actions);
            console(ui, model, theme, actions);
        });
}

/// From this width the subsystem cards sit two to a row.
const TWO_COLUMNS: f32 = 760.0;

/// The console card's height for its lines.
const CONSOLE_HEIGHT: f32 = 280.0;

/// A card's frame and its title row, with a status chip at the right when
/// there is one; `body` draws under the title.
fn card_frame(
    ui: &mut Ui,
    theme: &Palette,
    title: &str,
    tone: Option<(Tone, &str)>,
    extra: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) {
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(title.to_uppercase())
                        .size(10.5)
                        .strong()
                        .color(theme::rgb(theme.text_muted)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    extra(ui);
                    if let Some((tone, word)) = tone {
                        widgets::status_chip(ui, tone.icon(), word, tone.colour(theme));
                    }
                });
            });
            body(ui);
        });
    ui.add_space(SPACE_XS);
}

/// One subsystem's card: a title, a status chip, and its facts.
fn card(
    ui: &mut Ui,
    theme: &Palette,
    title: &str,
    tone: Option<(Tone, &str)>,
    facts: &[(&str, String)],
) {
    card_frame(
        ui,
        theme,
        title,
        tone,
        |_| {},
        |ui| {
            for (key, value) in facts {
                widgets::kv_row(ui, theme, key, value, theme.text);
            }
        },
    );
}

/// Below this the input counts as hearing nothing: a microphone's own
/// noise floor sits around -50 to -45 dBFS, and quiet music well above -40.
const SILENT_DBFS: f32 = -55.0;

/// The live input's level in dBFS, from its raw (not auto-gained) RMS, and
/// whether it hears anything. The levels the scenes see adapt to whatever
/// arrives, so "live, 120 BPM" alone looks the same on a dead microphone
/// (AMD desktop, Task A2).
fn input_level(live: bool, rms: f32) -> String {
    if !live {
        return "not capturing".to_string();
    }
    if !rms.is_finite() || rms <= 0.0 {
        return "no signal".to_string();
    }
    let dbfs = 20.0 * rms.log10();
    if dbfs < SILENT_DBFS {
        format!("{dbfs:.0} dBFS: hears nothing (check the device and its level)")
    } else {
        format!("{dbfs:.0} dBFS")
    }
}

/// The subsystem cards: renderer and script, audio and output, two to a row
/// from [`TWO_COLUMNS`].
fn cards(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    if ui.available_width() >= TWO_COLUMNS {
        ui.columns(2, |cols| {
            if let [left, right] = cols {
                renderer_card(left, model, theme);
                script_card(left, model, theme);
                audio_card(right, model, theme);
                output_card(right, model, theme);
            }
        });
    } else {
        renderer_card(ui, model, theme);
        audio_card(ui, model, theme);
        script_card(ui, model, theme);
        output_card(ui, model, theme);
    }
}

fn renderer_card(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    let perf = &model.perf;
    let gpu_tone = match (perf.has_timer, perf.gpu_ms) {
        (false, _) => (Tone::Warning, "no timer"),
        (true, None) => (Tone::Info, "measuring"),
        (true, Some(ms)) if ms > perf.budget_ms => (Tone::Warning, "over budget"),
        (true, Some(_)) => (Tone::Info, "in budget"),
    };
    let gpu = match perf.gpu_ms {
        Some(ms) => format!("{ms:.2} ms of {:.2} ms", perf.budget_ms),
        None => format!("budget {:.2} ms", perf.budget_ms),
    };
    card(
        ui,
        theme,
        "Renderer",
        Some(gpu_tone),
        &[
            ("Backend", model.gl_summary.to_string()),
            ("GPU time", gpu),
            (
                "Render scale",
                format!(
                    "{:.0}%{}",
                    perf.scale * 100.0,
                    if perf.adaptive && perf.has_timer {
                        " (adaptive)"
                    } else {
                        ""
                    }
                ),
            ),
            (
                "Render size",
                model.preview.map_or_else(
                    || "none".to_string(),
                    |p| format!("{}x{}", p.size[0], p.size[1]),
                ),
            ),
            (
                "Frame",
                format!(
                    "{:.1} fps, CPU {:.2} ms",
                    model.stats.fps(),
                    model.stats.cpu_ms()
                ),
            ),
            (
                "Control UI",
                format!("{:.2} ms layout per pass (30 Hz)", model.stats.ui_ms()),
            ),
        ],
    );
}

fn audio_card(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    let audio_tone = if model.audio.live {
        (Tone::Info, "live")
    } else if model.audio.can_capture {
        (Tone::Warning, "no input")
    } else {
        (Tone::Info, "synthetic")
    };
    card(
        ui,
        theme,
        "Audio",
        Some(audio_tone),
        &[
            ("Source", model.audio.source.to_string()),
            (
                "Input",
                input_level(model.audio.live, model.audio.input_rms),
            ),
            ("Tempo", format!("{:.1} BPM", model.audio.bpm)),
            (
                "Effect",
                if model.audio.live {
                    "visuals react to the input".to_string()
                } else {
                    "visuals run on the fixed 120 BPM track".to_string()
                },
            ),
        ],
    );
}

fn script_card(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    let (script_tone, script_text) = match model.script {
        None => (None, "no script".to_string()),
        Some(ScriptState::Running) => (Some((Tone::Info, "running")), "running".to_string()),
        Some(ScriptState::Faulted {
            consecutive,
            message,
        }) => (
            Some((Tone::Warning, "fault")),
            format!("{consecutive} in a row: {message}"),
        ),
        Some(ScriptState::Disabled { message }) => (
            Some((Tone::Error, "disabled")),
            format!("disabled: {message}"),
        ),
    };
    card(
        ui,
        theme,
        "Script",
        script_tone,
        &[("script.js", script_text)],
    );
}

fn output_card(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    let output = if !model.output_visible {
        "hidden (starts hidden; the scene keeps rendering)".to_string()
    } else if model.fullscreen {
        "shown, fullscreen".to_string()
    } else {
        "shown, windowed".to_string()
    };
    let monitor = model
        .output_monitor
        .and_then(|i| model.monitors.get(i))
        .cloned()
        .unwrap_or_else(|| "not chosen".to_string());
    card(
        ui,
        theme,
        "Output",
        Some(if model.output_visible {
            (Tone::Info, "shown")
        } else {
            (Tone::Info, "hidden")
        }),
        &[
            ("Window", output),
            ("Monitor", monitor),
            (
                "Blackout",
                if model.transport.blackout {
                    "on"
                } else {
                    "off"
                }
                .to_string(),
            ),
        ],
    );
}

/// The scenes that failed to load this run, as a card: a row each with
/// the scene's name, the first line of why, and Open in Editor.
fn needs_work(ui: &mut Ui, model: &Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let failed: Vec<(&str, &str)> = model
        .library
        .entries
        .iter()
        .zip(model.health)
        .filter_map(|(entry, health)| match health {
            Health::Failed(reason) => Some((entry.name.as_str(), reason.as_str())),
            Health::Unknown | Health::Ready => None,
        })
        .collect();
    let count = match failed.len() {
        0 => "none".to_string(),
        1 => "1 scene".to_string(),
        n => format!("{n} scenes"),
    };
    let tone = if failed.is_empty() {
        Tone::Info
    } else {
        Tone::Error
    };
    card_frame(
        ui,
        theme,
        "Needs work",
        Some((tone, &count)),
        |_| {},
        |ui| {
            if failed.is_empty() {
                ui.label(
                    egui::RichText::new("No scene has failed to load this run.")
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                );
                return;
            }
            for (index, (name, reason)) in failed.into_iter().enumerate() {
                if index > 0 {
                    widgets::section_rule(ui, theme);
                }
                ui.push_id(name, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(28.0);
                        ui.label(
                            egui::RichText::new(name)
                                .strong()
                                .color(theme::rgb(theme.text)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, theme, Some(Icon::Editor), "Open in Editor")
                                .clicked()
                            {
                                actions.push(Action::OpenEditor(Some(name.to_string())));
                            }
                        });
                    });
                    let why = reason.lines().next().unwrap_or_default();
                    let galley = widgets::truncated(
                        ui,
                        why,
                        theme::description_font(),
                        theme.text_muted,
                        ui.available_width(),
                    );
                    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                    ui.painter()
                        .galley(rect.min, galley, theme::rgb(theme.text_muted));
                    ui.add_space(SPACE_XS);
                })
                .response
                .on_hover_text(reason);
            }
        },
    );
}

/// The console, as a card [`CONSOLE_HEIGHT`] high whose lines scroll
/// inside it and stay at the newest.
fn console(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let only_problems = model.ui.console_problems_only;
    let mut clear = false;
    let mut toggle = false;
    card_frame(
        ui,
        theme,
        "Console",
        None,
        |ui| {
            clear = widgets::button(ui, theme, None, "Clear").clicked();
            toggle = widgets::tool_button(
                ui,
                theme,
                Icon::Warning,
                "Problems only",
                only_problems,
                "Show only warnings and errors",
            )
            .clicked();
        },
        |ui| {
            ui.add_space(SPACE_XS);
            let mut shown = 0_usize;
            egui::ScrollArea::vertical()
                .id_salt("diagnostics-console")
                .max_height(CONSOLE_HEIGHT)
                .min_scrolled_height(CONSOLE_HEIGHT)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in model.console.lines() {
                        let text = egui::RichText::new(&line.text).monospace().size(11.5);
                        let text = match line.level {
                            Level::Info => {
                                if only_problems {
                                    continue;
                                }
                                text.color(theme::rgb(theme.text))
                            }
                            Level::Debug => {
                                if only_problems {
                                    continue;
                                }
                                text.color(theme::rgb(theme.text_muted))
                            }
                            Level::Warn => text.color(theme::rgb(theme.warning)),
                            Level::Error => text.color(theme::rgb(theme.danger)),
                        };
                        ui.label(text);
                        shown += 1;
                    }
                    if shown == 0 {
                        ui.label(
                            egui::RichText::new(if only_problems {
                                "No warnings or errors."
                            } else {
                                "The console is empty."
                            })
                            .font(theme::description_font())
                            .color(theme::rgb(theme.text_muted)),
                        );
                    }
                });
        },
    );
    if clear {
        actions.push(Action::ClearConsole);
    }
    if toggle {
        model.ui.console_problems_only = !only_problems;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_input_level_tells_a_dead_microphone_from_a_live_one() -> Result<()> {
        assert_eq!(input_level(false, 0.5), "not capturing", "synthetic audio");
        assert_eq!(input_level(true, 0.0), "no signal", "nothing arrives");
        assert_eq!(input_level(true, f32::NAN), "no signal", "a bad value");
        let music = input_level(true, 0.1);
        assert_eq!(music, "-20 dBFS", "music at -20 dBFS");
        // The AMD desktop's silent microphone measured about -47 dBFS RMS
        // in winmm's 8-bit capture; at this module's resolution its floor
        // reads lower, around -60.
        let dead = input_level(true, 0.001);
        assert!(dead.contains("hears nothing"), "{dead}");
        Ok(())
    }
}
