// ============================================================================
// Module:       strobe_app::ui::lan
// Description:  Settings' Remote section: switch the LAN remote on and off,
//               its address, pairing with a code and Approve/Deny, and the
//               paired devices with Revoke.
//
// Dependencies: egui; super::{model, theme, widgets}; crate::{icon, lan,
//               palette}
// ============================================================================

//! The LAN remote's settings (L-03, L-06, D254) and the form for controlling
//! another Strobe desktop (D275).
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
}

/// Settings' "Control another Strobe" section, heading included: the form that makes this
/// desktop the controller of another one's whole window (D275).
///
/// It is its own section, with the command palette's "Control another
/// Strobe..." entry to open it, because it had been the last paragraph
/// of Remote, which is about being controlled: nobody looking to control a
/// second laptop would think to look there. `reveal` scrolls to it and
/// focuses the address field.
pub fn control(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let reveal = panel.controller.take_reveal();
    let section = ui.scope(|ui| {
        widgets::section(ui, theme, "Control another Strobe");
        ui.label(
            egui::RichText::new(
                "Operate another computer's whole Strobe window from here. On the host: Settings \
                 > Remote, Start remote, tick Allow full desktop control, then Pair a device to \
                 get a six-digit code. Type its address and that code below.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(SPACE_XS);
        let field = ui.horizontal_wrapped(|ui| {
            let address = ui.add(
                egui::TextEdit::singleline(&mut panel.controller.address)
                    .hint_text("Host address, e.g. 192.168.1.20")
                    .desired_width(230.0),
            );
            if reveal {
                address.request_focus();
            }
            ui.add(
                egui::TextEdit::singleline(&mut panel.controller.code)
                    .password(true)
                    .hint_text("Pairing code")
                    .desired_width(110.0),
            );
            if widgets::button(ui, theme, None, "Connect to desktop").clicked() {
                actions.push(Action::ConnectDesktop);
            }
            address.rect
        });
        ui.label(
            egui::RichText::new(
                "The port is 47812 unless the host says otherwise. Leave the code blank to \
                 reconnect to a host you paired with earlier in this session.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        let note = panel.controller.note();
        if !note.is_empty() {
            ui.label(note);
        }
        field.inner
    });
    if reveal {
        // The heading down to the focused field, or, where the pane is too
        // short for both (a small window), the field itself.
        let wanted = section.response.rect.with_max_y(section.inner.max.y);
        if wanted.height() <= ui.clip_rect().height() {
            ui.scroll_to_rect(wanted, Some(egui::Align::TOP));
        } else {
            ui.scroll_to_rect(section.inner, Some(egui::Align::Center));
        }
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
            "{} paired device{} connected. Windows may ask whether Strobe may accept connections: \
             allow it on private networks only.",
            view.connected,
            if view.connected == 1 { "" } else { "s" }
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
