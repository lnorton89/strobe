// ============================================================================
// Module:       strobe_app::ui::lan
// Description:  Settings' Remote section: switch the LAN remote on and off,
//               its address, pairing with a code and Approve/Deny, and the
//               paired devices with Revoke.
//
// Dependencies: egui; super::{model, theme, widgets}; crate::{icon, lan,
//               palette}
// ============================================================================

//! The LAN remote's settings (L-03, L-06, D254).
//!
//! The section reads a [`LanView`](crate::lan::LanView) the app made this
//! frame and hands back [`Action`]s; it never touches the listener. The
//! order is the order a person needs it in: whether it is on and where,
//! then pairing (a code to type on the phone, then the phone to approve),
//! then who is paired. Revoking works with the remote off. The title bar
//! carries the always-visible "Remote on" readout and its one-click Stop
//! (`super::chrome`).

use super::model::{Action, LanPanel};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use egui::Ui;

/// Draws the section.
pub fn show(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    ui.label(
        egui::RichText::new(
            "Run the show from a phone, tablet or another Strobe desktop on the same network. Off until you switch it \
             on, and never on by itself at launch. Plain HTTP: use it only on a network you \
             trust.",
        )
        .small()
        .color(theme::rgb(theme.text_muted)),
    );
    ui.add_space(SPACE_XS);
    if view.running {
        running(ui, theme, panel, actions);
    } else {
        stopped(ui, theme, panel, actions);
    }
    ui.add_space(SPACE_SM);
    paired(ui, theme, panel, actions);
    ui.add_space(SPACE_SM);
    let mut desktop_enabled = panel.desktop_enabled;
    if ui
        .add_enabled(
            view.running,
            egui::Checkbox::new(&mut desktop_enabled, "Allow full desktop control"),
        )
        .changed()
    {
        actions.push(Action::SetDesktopAccess(desktop_enabled));
    }
    ui.label(egui::RichText::new("Grants desktops you approve the entire Strobe UI, editor and file saves included, but not pairing or network settings. One controller at a time. Switching it off, or stopping the remote, ends every desktop pairing.").small().color(theme::rgb(theme.text_muted)));
    if view.venue {
        ui.label(
            egui::RichText::new(
                "Venue mode: this Strobe was started with a trust file. While full desktop \
                 control is on, a controller that holds the same file gets full control with no \
                 approval here. Revoke ends its session and refuses that controller until Strobe \
                 restarts; Revoke all, or switching full desktop control off, closes the door.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
    }
    ui.add_space(SPACE_SM);
    ui.strong("Control another Strobe desktop");
    ui.label("Start remote and allow full desktop control on the host, then open pairing there.");
    ui.horizontal_wrapped(|ui| {
        ui.label("Host IP:port");
        ui.add(
            egui::TextEdit::singleline(&mut panel.controller.address)
                .hint_text("192.168.1.20:47812")
                .desired_width(180.0),
        );
        ui.label("Pairing code");
        ui.add(
            egui::TextEdit::singleline(&mut panel.controller.code)
                .password(true)
                .desired_width(90.0),
        );
        if widgets::button(ui, theme, None, "Connect to desktop").clicked() {
            actions.push(Action::ConnectDesktop);
        }
    });
    ui.label(
        egui::RichText::new(
            "Leave the code blank to reconnect to the same host during this app session.",
        )
        .small()
        .color(theme::rgb(theme.text_muted)),
    );
    let note = panel.controller.note();
    if !note.is_empty() {
        ui.label(note);
    }
}

fn stopped(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Address");
        ui.add(
            egui::TextEdit::singleline(panel.address)
                .hint_text("this computer's LAN address")
                .desired_width(150.0),
        )
        .on_hover_text(
            "An IP address of this computer. Empty uses its LAN address. A public address is \
             refused; 0.0.0.0 listens on every interface.",
        );
        ui.label("Port");
        ui.add(egui::TextEdit::singleline(panel.port).desired_width(56.0));
    });
    ui.add_space(SPACE_XS);
    if widgets::primary_button(ui, theme, "Start remote").clicked() {
        actions.push(Action::SetLanRemote(true));
    }
    note(ui, theme, panel);
}

fn note(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>) {
    let view = panel.view;
    if view.failed {
        widgets::banner(ui, theme, Tone::Error, &view.note, |_| {});
    } else {
        ui.label(
            egui::RichText::new(&view.note)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
}

fn running(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    let url = view.url.as_deref().unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        widgets::status_chip(ui, Icon::Check, "Remote on", theme.success);
        ui.label(egui::RichText::new(url).monospace().strong());
        if widgets::button(ui, theme, Some(Icon::Copy), "Copy")
            .on_hover_text("Open this address in the phone's browser. It holds no secret.")
            .clicked()
        {
            ui.ctx().copy_text(url.to_string());
        }
    });
    ui.label(
        egui::RichText::new(format!(
            "{} connection{} open. Windows may ask whether Strobe may accept connections: allow \
             it on private networks only.",
            view.connections,
            if view.connections == 1 { "" } else { "s" }
        ))
        .small()
        .color(theme::rgb(theme.text_muted)),
    );
    if let Some(warning) = &view.warning {
        widgets::banner(ui, theme, Tone::Warning, warning, |_| {});
    }
    ui.add_space(SPACE_XS);
    if widgets::button(ui, theme, Some(Icon::Close), "Stop remote")
        .on_hover_text("Stops listening and drops every phone at once.")
        .clicked()
    {
        actions.push(Action::SetLanRemote(false));
    }
    ui.add_space(SPACE_SM);
    pairing(ui, theme, panel, actions);
}

fn pairing(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    match &view.code {
        Some((code, left)) => {
            ui.label("Type this code on the phone:");
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(code)
                        .monospace()
                        .size(28.0)
                        .strong()
                        .color(theme::rgb(theme.accent)),
                );
                ui.label(
                    egui::RichText::new(format!("expires in {}:{:02}", left / 60, left % 60))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
                if widgets::button(ui, theme, None, "Close pairing").clicked() {
                    actions.push(Action::CloseRemotePairing);
                }
            });
        }
        None => {
            if widgets::button(ui, theme, Some(Icon::Info), "Pair a device")
                .on_hover_text(
                    "Shows a six-digit code for two minutes. The phone types it, then you \
                     approve the phone here.",
                )
                .clicked()
            {
                actions.push(Action::OpenRemotePairing);
            }
        }
    }
    for request in &view.requests {
        ui.add_space(SPACE_XS);
        widgets::banner(
            ui,
            theme,
            Tone::Warning,
            &if request
                .label
                .starts_with(strobe_remote::desktop::LABEL_PREFIX)
            {
                format!("{} wants full control, files included.", request.label)
            } else {
                format!("{} wants to control the show.", request.label)
            },
            |ui| {
                if widgets::primary_button(ui, theme, "Approve").clicked() {
                    actions.push(Action::DecideRemotePairing {
                        id: request.id,
                        approve: true,
                    });
                }
                if widgets::button(ui, theme, None, "Deny").clicked() {
                    actions.push(Action::DecideRemotePairing {
                        id: request.id,
                        approve: false,
                    });
                }
            },
        );
    }
}

fn paired(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    ui.label(egui::RichText::new("Paired devices").strong());
    if view.clients.is_empty() {
        ui.label(
            egui::RichText::new("None.")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        return;
    }
    for (client, connected) in &view.clients {
        ui.horizontal_wrapped(|ui| {
            ui.label(&client.label);
            if client
                .label
                .starts_with(strobe_remote::desktop::TRUSTED_LABEL_PREFIX)
            {
                widgets::status_chip(ui, Icon::Info, "trusted", theme.warning);
            }
            if *connected {
                widgets::status_chip(ui, Icon::Check, "connected", theme.success);
            }
            if widgets::button(ui, theme, Some(Icon::Close), "Revoke")
                .on_hover_text(
                    "This device must pair again. If it is connected, it is dropped now.",
                )
                .clicked()
            {
                actions.push(Action::RevokeRemoteClient(client.id.clone()));
            }
        });
    }
    ui.add_space(SPACE_XS);
    if widgets::button(ui, theme, Some(Icon::Reset), "Revoke all").clicked() {
        actions.push(Action::RevokeAllRemoteClients);
    }
}
