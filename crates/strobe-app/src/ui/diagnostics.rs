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

use super::model::{Action, Model};
use super::state::UiState;
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
        .id_salt("diagnostics-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            cards(ui, model, theme);
            needs_work(ui, model, theme);
            console(ui, model, theme, actions);
        });
}

/// One subsystem's card: a title, a status chip, and its facts.
fn card(
    ui: &mut Ui,
    theme: &Palette,
    title: &str,
    tone: Option<(Tone, &str)>,
    facts: &[(&str, String)],
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
                if let Some((tone, word)) = tone {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::status_chip(ui, tone.icon(), word, tone.colour(theme));
                    });
                }
            });
            for (key, value) in facts {
                widgets::kv_row(ui, theme, key, value, theme.text);
            }
        });
    ui.add_space(SPACE_XS);
}

fn cards(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
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
        ],
    );

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

/// The scenes that failed to load, with the first line of why.
fn needs_work(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
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
    widgets::section(ui, theme, "Needs work");
    if failed.is_empty() {
        ui.label(
            egui::RichText::new("No scene has failed to load this run.")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        return;
    }
    for (name, reason) in failed {
        widgets::banner(
            ui,
            theme,
            Tone::Error,
            &format!("{name}: {}", reason.lines().next().unwrap_or("")),
            |_| {},
        );
        ui.add_space(SPACE_XS);
    }
}

fn console(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    ui.add_space(SPACE_SM);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("CONSOLE")
                .size(10.5)
                .strong()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, theme, None, "Clear").clicked() {
                actions.push(Action::ClearConsole);
            }
            let problems = problems_only(ui, model.ui);
            let _ = problems;
        });
    });
    widgets::section_rule(ui, theme);
    ui.add_space(SPACE_XS);
    let only_problems = model.ui.console_problems_only;
    let mut shown = 0_usize;
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
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
    }
}

/// The "problems only" switch.
fn problems_only(ui: &mut Ui, state: &mut UiState) -> bool {
    let theme = super::theme::palette();
    let on = state.console_problems_only;
    if widgets::tool_button(
        ui,
        &theme,
        Icon::Warning,
        "Problems only",
        on,
        "Show only warnings and errors",
    )
    .clicked()
    {
        state.console_problems_only = !on;
    }
    state.console_problems_only
}
