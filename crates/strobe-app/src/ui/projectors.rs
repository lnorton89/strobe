// ============================================================================
// Module:       strobe_app::ui::projectors
// Description:  Settings' Projectors page: switch projector control on, each
//               projector's power and status with On and Off, the web page
//               a phone opens, and the connections (addresses, passwords,
//               NTCONTROL fallback, the page's bind and port).
//
// Dependencies: egui; super::{form, model, theme, widgets}; crate::{icon,
//               palette, projectors}
// ============================================================================

//! Projector power (D502), in `super::form`'s grammar.
//!
//! - **Control**: one switch. Off, nothing talks to a projector.
//! - **Projectors**: one row per saved projector, its power as a chip and
//!   its input, hours and errors under it, with On and Off. Off asks twice:
//!   the first press turns the button into "Confirm off", as the web page
//!   asks before switching off. "All projectors" switches every one.
//! - **Web page**: serve the page from Strobe, and its address with Copy
//!   link. A phone opens it and signs in with the page's password (D516).
//! - **Connections**: the projectors' settings, edited in a copy and saved
//!   together, where the page listens, and the page's password.
//!
//! The groups read a [`ProjectorView`](crate::projectors::ProjectorView)
//! the app made this frame and edit the form; they hand back [`Action`]s
//! and never touch a projector.

use super::form::{self, Row};
use super::model::{Action, ProjectorPanel};
use super::theme;
use super::widgets::{self, Segment, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::projectors::{Form, UnitView};
use egui::Ui;
use strobe_projector::config::Bind;

// Row titles, also their entries in Settings' search.

/// The switch that turns projector control on.
pub const CONTROL: &str = "Projector control";
/// The switch that serves the web page.
pub const WEB: &str = "Web page";
/// The row that adds a projector.
pub const ADD: &str = "Add a projector";
/// Where the page listens.
pub const LISTEN: &str = "Listen on";
/// The page's password (D516).
pub const PASSWORD: &str = "Page password";
/// The row that switches every projector.
pub const ALL: &str = "All projectors";

/// The width of a text field in a projector's settings.
const FIELD: f32 = 220.0;

/// Draws the page.
pub fn show(
    ui: &mut Ui,
    theme: &Palette,
    panel: &mut ProjectorPanel<'_>,
    actions: &mut Vec<Action>,
) {
    let view = panel.view;
    if let Some(error) = &view.load_error {
        ui.add_space(theme::SPACE_MD);
        widgets::banner(ui, theme, Tone::Error, error, |_| {});
    }
    form::group(ui, theme, "Control", |group| {
        let (icon, word, colour) = if view.enabled {
            (Icon::Check, "On", theme.success)
        } else {
            (Icon::Info, "Off", theme.text_muted)
        };
        let mut on = view.enabled;
        let row = Row::new(CONTROL).status(icon, word, colour).description(
            "Switch the projectors on and off over the network: PJLink, with Panasonic's \
             NTCONTROL when PJLink fails. Status is read every 30 s while this is on.",
        );
        if group.toggle(row, &mut on).changed() {
            actions.push(Action::SetProjectorControl(on));
        }
    });
    if view.enabled {
        units(ui, theme, panel, actions);
    }
    web(ui, theme, panel, actions);
    connections(ui, theme, panel.form, view.dirty, &view.path, actions);
}

/// The chip for a unit's power state.
fn power_chip(theme: &Palette, unit: &UnitView) -> (Icon, String, crate::color::Rgb) {
    if unit.busy {
        return (Icon::Refresh, "working".into(), theme.warning);
    }
    let Some(status) = &unit.status else {
        return (Icon::Info, "unknown".into(), theme.text_muted);
    };
    match status.power.as_str() {
        "on" => (Icon::Check, "on".into(), theme.success),
        "off" => (Icon::Blackout, "off".into(), theme.text_muted),
        "warming" | "cooling" => (Icon::Refresh, status.power.clone(), theme.warning),
        _ if !status.ok => (Icon::Error, "error".into(), theme.danger),
        other => (Icon::Info, other.to_string(), theme.text_muted),
    }
}

/// The line under a unit's name: its address and what it reported.
fn details(unit: &UnitView) -> String {
    let mut parts = vec![unit.ip.clone()];
    if let Some(status) = &unit.status {
        if !status.ok {
            parts.push(status.power.clone());
        }
        for (label, value) in [
            ("Input", &status.input),
            ("Hours", &status.hours),
            ("Errors", &status.errors),
        ] {
            if let Some(value) = value {
                parts.push(format!("{label}: {value}"));
            }
        }
        if let Some(via) = status.via {
            parts.push(format!("via {}", via.name()));
        }
    }
    parts.join(" · ")
}

/// On and Off for `target`; Off needs a second press.
fn power_buttons(
    ui: &mut Ui,
    theme: &Palette,
    form: &mut Form,
    target: &str,
    busy: bool,
    actions: &mut Vec<Action>,
) {
    ui.add_enabled_ui(!busy, |ui| {
        let confirming = form.confirm_off.as_deref() == Some(target);
        // The column lays out right to left: Off is the rightmost.
        let off = if confirming {
            widgets::primary_button(ui, theme, "Confirm off")
        } else {
            widgets::button(ui, theme, Some(Icon::Blackout), "Off")
        };
        if off.clicked() {
            if confirming {
                actions.push(Action::ProjectorPower {
                    target: target.to_string(),
                    on: false,
                });
            } else {
                form.confirm_off = Some(target.to_string());
            }
        }
        if widgets::button(ui, theme, Some(Icon::Check), "On").clicked() {
            form.confirm_off = None;
            actions.push(Action::ProjectorPower {
                target: target.to_string(),
                on: true,
            });
        }
    });
}

fn units(ui: &mut Ui, theme: &Palette, panel: &mut ProjectorPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    form::group(ui, theme, "Projectors", |group| {
        if view.units.is_empty() {
            group.block(|ui| {
                ui.label(
                    egui::RichText::new("No projectors yet: add one under Connections.")
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                );
            });
            return;
        }
        for unit in &view.units {
            let (icon, word, colour) = power_chip(theme, unit);
            let line = details(unit);
            group.row(
                Row::new(&unit.name)
                    .status(icon, &word, colour)
                    .description(&line),
                |ui| power_buttons(ui, theme, panel.form, &unit.key, unit.busy, actions),
            );
        }
        let any_busy = view.units.iter().any(|unit| unit.busy);
        let checked = if view.polling {
            "Reading status..."
        } else {
            "Switches every projector at once."
        };
        group.row(Row::new(ALL).description(checked), |ui| {
            power_buttons(ui, theme, panel.form, "all", any_busy, actions);
            if widgets::icon_button(ui, theme, Icon::Refresh, "Read status now").clicked() {
                actions.push(Action::RefreshProjectors);
            }
        });
        if let Some((message, failed)) = &view.message {
            group.block(|ui| {
                let colour = if *failed {
                    theme.danger
                } else {
                    theme.text_muted
                };
                ui.label(
                    egui::RichText::new(message)
                        .font(theme::description_font())
                        .color(theme::rgb(colour)),
                );
            });
        }
    });
}

fn web(ui: &mut Ui, theme: &Palette, panel: &ProjectorPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    form::group(ui, theme, "Web page", |group| {
        let (icon, word, colour) = if view.listening {
            (Icon::Check, "Serving", theme.success)
        } else if view.serve_web && view.enabled {
            (Icon::Warning, "Waiting", theme.warning)
        } else {
            (Icon::Info, "Off", theme.text_muted)
        };
        let mut on = view.serve_web;
        let mut row = Row::new(WEB)
            .enabled(view.enabled)
            .status(icon, word, colour)
            .description(
                "A page with On and Off for each projector, for a phone on the venue Wi-Fi or \
                 the tailnet. It listens only on the Tailscale or Wi-Fi address, and asks \
                 for the page's password (under Connections). Plain HTTP: never forward its \
                 port.",
            );
        if !view.enabled {
            row = row.note("Turn projector control on first.", theme.text_muted);
        } else if view.serve_web && !view.listening {
            row = row.note(&view.web_state, theme.warning);
        }
        if group.toggle(row, &mut on).changed() && view.enabled {
            actions.push(Action::SetProjectorWeb(on));
        }
        if let Some(url) = &view.url {
            group.row(
                Row::new("Address")
                    .status(Icon::Check, url, theme.success)
                    .description(
                        "Open it on a phone and sign in with the page's password; the phone \
                         stays signed in until the password changes.",
                    ),
                |ui| {
                    if widgets::button(ui, theme, Some(Icon::Copy), "Copy link").clicked() {
                        ui.ctx().copy_text(url.clone());
                    }
                },
            );
        }
    });
}

/// A password field: dots, unless the field is being edited.
fn secret(ui: &mut Ui, theme: &Palette, text: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(text)
            .password(true)
            .hint_text(egui::RichText::new(hint).color(theme::rgb(theme.text_faint)))
            .desired_width(FIELD)
            .margin(theme::margin_xy(theme::SPACE_SM, theme::SPACE_XS)),
    );
}

fn connections(
    ui: &mut Ui,
    theme: &Palette,
    form: &mut Form,
    dirty: bool,
    path: &str,
    actions: &mut Vec<Action>,
) {
    form::group(ui, theme, "Connections", |group| {
        let mut remove = None;
        let mut toggle = None;
        for index in 0..form.units.len() {
            let Some(unit) = form.units.get(index) else {
                break;
            };
            let title = if unit.projector.name.is_empty() {
                unit.key.clone()
            } else {
                unit.projector.name.clone()
            };
            let fallback = if unit.projector.ntcontrol_fallback() {
                ", NTCONTROL fallback"
            } else {
                ""
            };
            let line = format!("{} · key {}{fallback}", unit.projector.ip, unit.key);
            let open = form.open == Some(index);
            group.row(Row::new(&title).description(&line), |ui| {
                if widgets::button(ui, theme, Some(Icon::Close), "Remove").clicked() {
                    remove = Some(index);
                }
                let label = if open { "Done" } else { "Edit" };
                if widgets::button(ui, theme, None, label).clicked() {
                    toggle = Some(index);
                }
            });
            if open {
                group.block(|ui| {
                    if let Some(unit) = form.units.get_mut(index) {
                        edit_unit(ui, theme, unit);
                    }
                });
            }
        }
        if let Some(index) = toggle {
            form.open = (form.open != Some(index)).then_some(index);
        }
        if let Some(index) = remove {
            form.remove(index);
        }
        group.row(
            Row::new(ADD).description("A projector on the projector network, by its address."),
            |ui| {
                if widgets::button(ui, theme, Some(Icon::Plus), "Add").clicked() {
                    form.add();
                }
            },
        );
        group.row(
            Row::new(LISTEN).description(
                "Auto: the Tailscale address when Tailscale is up, else the Wi-Fi address. \
                 The port is the page's.",
            ),
            |ui| {
                form::field(ui, theme, &mut form.port, "8780", 56.0);
                let segments: [Segment<'_>; 3] = [
                    (Icon::Star, "Auto", "Tailscale when it is up, else Wi-Fi"),
                    (Icon::Mapping, "Tailscale", "Only the tailnet address"),
                    (Icon::Output, "Wi-Fi", "Only the Wi-Fi address"),
                ];
                let active = Bind::ALL
                    .iter()
                    .position(|bind| *bind == form.bind)
                    .unwrap_or_default();
                if let Some(chosen) =
                    widgets::segmented(ui, theme, "projector-bind", &segments, active, 72.0)
                    && let Some(bind) = Bind::ALL.get(chosen)
                {
                    form.bind = *bind;
                }
            },
        );
        group.row(
            Row::new(PASSWORD).description(
                "What a phone types to open the web page, at least 6 characters. Changing it \
                 signs every phone out.",
            ),
            |ui| secret(ui, theme, &mut form.password, "none: the page stays shut"),
        );
        let saved = if dirty {
            "Unsaved changes. Save checks every projector first."
        } else {
            path
        };
        group.row(Row::new("Save").enabled(dirty).description(saved), |ui| {
            ui.add_enabled_ui(dirty, |ui| {
                if widgets::primary_button(ui, theme, "Save").clicked() {
                    actions.push(Action::SaveProjectors);
                }
                if widgets::button(ui, theme, None, "Revert").clicked() {
                    actions.push(Action::RevertProjectors);
                }
            });
        });
    });
}

/// One projector's settings, opened under its row.
fn edit_unit(ui: &mut Ui, theme: &Palette, unit: &mut crate::projectors::DraftUnit) {
    form::inset(ui, theme, |inset| {
        let projector = &mut unit.projector;
        inset.row(
            Row::new("Name").description("What the page and this list call it."),
            |ui| {
                form::field(ui, theme, &mut projector.name, "Panasonic PT-DZ6700", FIELD);
            },
        );
        inset.row(
            Row::new("Key")
                .description("Letters, digits or _: the web API's and pj's name for it."),
            |ui| {
                form::field(ui, theme, &mut unit.key, "panasonic", FIELD);
            },
        );
        inset.row(
            Row::new("Address").description("Its IP on the projector network."),
            |ui| {
                form::field(ui, theme, &mut projector.ip, "192.168.50.10", FIELD);
            },
        );
        inset.row(
            Row::new("PJLink password")
                .description("Empty for none. Some units ask for one even with it shown off."),
            |ui| secret(ui, theme, &mut projector.pjlink_password, "none"),
        );
        let mut fallback = projector.ntcontrol_fallback();
        let row = Row::new("NTCONTROL fallback").description(
            "Panasonic only: when PJLink fails on this unit, switch it over Panasonic's own \
             protocol with the web interface's user and password.",
        );
        if inset.toggle(row, &mut fallback).changed() {
            projector.fallback = if fallback {
                "ntcontrol".into()
            } else {
                String::new()
            };
        }
        if fallback {
            inset.row(Row::new("Web user"), |ui| {
                form::field(ui, theme, &mut projector.ntcontrol_user, "admin1", FIELD);
            });
            inset.row(Row::new("Web password"), |ui| {
                secret(ui, theme, &mut projector.ntcontrol_password, "none");
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectors::ProjectorView;
    use anyhow::{Result, anyhow};
    use egui::{Pos2, Vec2};
    use strobe_projector::unit::Status;

    fn view() -> ProjectorView {
        ProjectorView {
            enabled: true,
            units: vec![UnitView {
                key: "sony".into(),
                name: "Sony VPL-PHZ10".into(),
                ip: "192.168.50.11".into(),
                status: Some(Status {
                    unit: "sony".into(),
                    power: "on".into(),
                    errors: Some("no errors".into()),
                    ok: true,
                    ..Status::default()
                }),
                busy: false,
            }],
            ..ProjectorView::default()
        }
    }

    /// Draws the Projectors group, with `events`, and returns every text
    /// drawn and where.
    fn draw(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        form: &mut Form,
        actions: &mut Vec<Action>,
    ) -> Vec<(String, egui::Rect)> {
        let theme = crate::palette::fallback().clone();
        let view = view();
        let pass = crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(720.0, 480.0), events),
            |ui| {
                let mut panel = ProjectorPanel {
                    view: &view,
                    form: &mut *form,
                };
                units(ui, &theme, &mut panel, actions);
            },
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

    fn find(drawn: &[(String, egui::Rect)], text: &str) -> Result<Pos2> {
        drawn
            .iter()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| rect.center())
            .ok_or_else(|| anyhow!("no {text:?} in {drawn:?}"))
    }

    fn click(ctx: &egui::Context, at: Pos2, form: &mut Form, actions: &mut Vec<Action>) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw(
            ctx,
            vec![egui::Event::PointerMoved(at), button(true)],
            form,
            actions,
        );
        draw(ctx, vec![button(false)], form, actions);
    }

    #[test]
    fn off_asks_twice_and_on_asks_once() -> Result<()> {
        let ctx = egui::Context::default();
        let mut form = Form::default();
        let mut actions = Vec::new();
        let drawn = draw(&ctx, Vec::new(), &mut form, &mut actions);
        assert!(
            drawn
                .iter()
                .any(|(text, _)| text.contains("Errors: no errors")),
            "the status shows: {drawn:?}"
        );
        click(&ctx, find(&drawn, "Off")?, &mut form, &mut actions);
        assert!(
            actions.is_empty(),
            "the first Off sends nothing: {actions:?}"
        );
        assert_eq!(form.confirm_off.as_deref(), Some("sony"), "it asks");
        let drawn = draw(&ctx, Vec::new(), &mut form, &mut actions);
        click(&ctx, find(&drawn, "Confirm off")?, &mut form, &mut actions);
        assert_eq!(
            actions,
            vec![Action::ProjectorPower {
                target: "sony".into(),
                on: false,
            }],
            "the second press sends it"
        );
        actions.clear();
        let drawn = draw(&ctx, Vec::new(), &mut form, &mut actions);
        click(&ctx, find(&drawn, "On")?, &mut form, &mut actions);
        assert_eq!(
            actions,
            vec![Action::ProjectorPower {
                target: "sony".into(),
                on: true,
            }],
            "On needs no confirmation"
        );
        Ok(())
    }
}
