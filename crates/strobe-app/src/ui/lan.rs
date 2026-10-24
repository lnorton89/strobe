// ============================================================================
// Module:       strobe_app::ui::lan
// Description:  Settings' Remote page: let others connect (the LAN remote,
//               found by other Strobes, each request approved or denied),
//               the paired devices, the Strobes on the network to control,
//               and the address and port.
//
// Dependencies: egui; super::{form, model, theme, widgets}; crate::{icon,
//               lan, palette}; strobe-remote (discovery)
// ============================================================================

//! The LAN remote (L-03, L-06, D254) and controlling another Strobe (D275),
//! in `super::form`'s grammar, streamlined (D468).
//!
//! One switch lets others connect. While it is on, other Strobes on the
//! network list this one by name, and anyone asking to connect is put in
//! front of the operator as a banner at the top of the page with Approve
//! and Deny. A desktop needs no code; a phone, which cannot list anything,
//! still pairs with the code from Pair a phone.
//!
//! The other way round, the Strobes found on the network are listed with a
//! Connect button: the host's operator approves, and the viewer takes over
//! the window. An address typed by hand, and a venue trust file, are the
//! fallbacks below the list.
//!
//! The groups read a [`LanView`](crate::lan::LanView) the app made this
//! frame, and the scanner's list, and hand back [`Action`]s; they never
//! touch the listener. The title bar carries the always-visible "Remote
//! on" readout and its one-click Stop (`super::chrome`), and clicking it
//! opens this page.

use super::form::{self, Row};
use super::model::{Action, LanPanel};
use super::theme;
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use egui::Ui;

// Row titles, also their entries in Settings' search.

/// The switch that lets others connect.
pub const REMOTE: &str = "Let others connect";
/// The phone pairing row.
pub const PAIR: &str = "Pair a phone";
/// The full-desktop-control switch.
pub const DESKTOP: &str = "Allow full desktop control";
/// The row that revokes every paired device.
pub const REVOKE_ALL: &str = "Revoke all";
/// Connecting to a Strobe that is not listed.
pub const HOST: &str = "Connect by address";
/// The venue-mode switch.
pub const USE_TRUST: &str = "Use a trust file (venue mode)";

/// The width of the address and path fields.
const FIELD: f32 = 200.0;

/// Draws the requests waiting for an answer, "This computer" and "Paired
/// devices".
pub fn show(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    requests(ui, theme, panel, actions);
    form::group(ui, theme, "This computer", |group| {
        let (icon, word, colour) = if view.running {
            (Icon::Check, "On", theme.success)
        } else {
            (Icon::Info, "Off", theme.text_muted)
        };
        let url = view.url.as_deref().unwrap_or_default();
        let description = if !view.running {
            "Lets phones and other Strobes on this network run the show. You approve each one. \
             Plain HTTP: use it only on a network you trust."
                .to_string()
        } else if view.advertising {
            format!(
                "Other Strobes on this network list this one as {}. You approve each one that \
                 asks.",
                view.name
            )
        } else {
            format!(
                "Other Strobes cannot find this one by themselves (another program holds the \
                 discovery port); connect from them by address: {url}"
            )
        };
        let mut row = Row::new(REMOTE)
            .status(icon, word, colour)
            .description(&description);
        if view.failed {
            row = row.note(&view.note, theme.danger);
        }
        let mut on = view.running;
        if group.toggle(row, &mut on).changed() {
            actions.push(Action::SetLanRemote(on));
        }
        let mut desktop_enabled = panel.desktop_enabled;
        let mut row = Row::new(DESKTOP).enabled(view.running).description(
            "Strobes you approve operate this whole window, editor and file saves included, but \
             not pairing or network settings. One at a time. Switching it off ends every \
             desktop pairing.",
        );
        if !view.running {
            row = row.note("Let others connect first.", theme.text_muted);
        } else if view.venue {
            row = row.note(
                "Venue mode: started with a trust file. A controller holding the same file gets \
                 full control with no approval here while this is on.",
                theme.warning,
            );
        }
        if group.toggle(row, &mut desktop_enabled).changed() {
            actions.push(Action::SetDesktopAccess(desktop_enabled));
        }
        if view.running {
            phones(group, theme, panel, actions);
        }
    });
    paired(ui, theme, panel, actions);
}

/// The requests waiting for Approve or Deny, as banners at the top of the
/// page, where they cannot be missed.
fn requests(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>, actions: &mut Vec<Action>) {
    for request in &panel.view.requests {
        let desktop = request
            .label
            .strip_prefix(strobe_remote::desktop::LABEL_PREFIX);
        let message = match desktop {
            Some(who) => format!("{who} wants to control this Strobe, files included."),
            None => format!("{} wants to control the show.", request.label),
        };
        ui.add_space(theme::SPACE_MD);
        widgets::banner(ui, theme, Tone::Warning, &message, |ui| {
            if widgets::button(ui, theme, None, "Deny").clicked() {
                actions.push(Action::DecideRemotePairing {
                    id: request.id,
                    approve: false,
                });
            }
            if widgets::primary_button(ui, theme, "Approve").clicked() {
                actions.push(Action::DecideRemotePairing {
                    id: request.id,
                    approve: true,
                });
            }
        });
    }
}

/// The phone rows, while the remote listens: the address a phone opens,
/// any warning about the bind, and pairing with a code.
fn phones(
    group: &mut form::Group<'_>,
    theme: &Palette,
    panel: &LanPanel<'_>,
    actions: &mut Vec<Action>,
) {
    let view = panel.view;
    let url = view.url.as_deref().unwrap_or_default();
    let connected = format!(
        "Open it in a phone's browser. {} paired device{} connected.",
        view.connected,
        if view.connected == 1 { "" } else { "s" }
    );
    group.row(
        Row::new("Phone address")
            .status(Icon::Check, url, theme.success)
            .description(&connected),
        |ui| {
            if widgets::button(ui, theme, Some(Icon::Copy), "Copy")
                .on_hover_text("It holds no secret.")
                .clicked()
            {
                ui.ctx().copy_text(url.to_string());
            }
        },
    );
    if let Some(warning) = &view.warning {
        group.block(|ui| widgets::banner(ui, theme, Tone::Warning, warning, |_| {}));
    }
    match &view.code {
        Some((code, left)) => {
            let expires = format!(
                "Type this code on the phone, then approve it here. It expires in {}:{:02}.",
                left / 60,
                left % 60
            );
            group.row(Row::new(PAIR).description(&expires), |ui| {
                if widgets::button(ui, theme, None, "Close").clicked() {
                    actions.push(Action::CloseRemotePairing);
                }
                ui.label(
                    egui::RichText::new(code)
                        .monospace()
                        .size(28.0)
                        .strong()
                        .color(theme::rgb(theme.accent)),
                );
            });
        }
        None => {
            group.row(
                Row::new(PAIR).description(
                    "A phone cannot find this Strobe by itself: it opens the address above and \
                     types a code shown here for two minutes.",
                ),
                |ui| {
                    if widgets::button(ui, theme, None, "Show a code").clicked() {
                        actions.push(Action::OpenRemotePairing);
                    }
                },
            );
        }
    }
}

fn paired(ui: &mut Ui, theme: &Palette, panel: &LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    form::group(ui, theme, "Paired devices", |group| {
        if view.clients.is_empty() {
            group.block(|ui| {
                ui.label(
                    egui::RichText::new("No paired devices yet.")
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                );
            });
            return;
        }
        for (client, connected) in &view.clients {
            let trusted = client
                .label
                .starts_with(strobe_remote::desktop::TRUSTED_LABEL_PREFIX);
            let shown = client
                .label
                .strip_prefix(strobe_remote::desktop::LABEL_PREFIX)
                .unwrap_or(&client.label);
            let mut row = Row::new(shown);
            if *connected {
                row = row.status(Icon::Check, "connected", theme.success);
            }
            if trusted {
                row = row.description("Trusted: holds this host's venue trust file.");
            } else if shown.len() != client.label.len() {
                row = row.description("Full desktop control.");
            }
            group.row(row, |ui| {
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
        group.row(
            Row::new(REVOKE_ALL).description("Every device must pair again."),
            |ui| {
                if widgets::button(ui, theme, Some(Icon::Reset), "Revoke all").clicked() {
                    actions.push(Action::RevokeAllRemoteClients);
                }
            },
        );
    });
}

/// "Control another Strobe": the Strobes found on the network, each with
/// Connect, then an address typed by hand and a venue trust file (D275,
/// D468).
///
/// It is its own group, with the command palette's "Control another
/// Strobe..." entry to open it: nobody looking to control a second laptop
/// would think to look under being controlled. `reveal` scrolls to the
/// address field, lights it and focuses it.
pub fn control(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let reveal = panel.controller.take_reveal();
    if reveal {
        form::reveal(HOST);
    }
    form::group(ui, theme, "Control another Strobe", |group| {
        if panel.found.is_empty() {
            group.block(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Looking for Strobes on this network. One appears here when Let \
                             others connect is on there.",
                        )
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                    )
                    .wrap(),
                );
            });
        }
        for found in panel.found {
            let (icon, word, colour) = if found.desktop {
                (Icon::Check, "Ready", theme.success)
            } else {
                (Icon::Info, "Desktop control off", theme.text_muted)
            };
            let address = found.addr.to_string();
            let description = if found.desktop {
                address.clone()
            } else {
                format!("{address}: its operator has not allowed full desktop control.")
            };
            let mut connect = false;
            group.row(
                Row::new(&found.name)
                    .status(icon, word, colour)
                    .description(&description),
                |ui| {
                    connect = ui
                        .add_enabled_ui(found.desktop, |ui| {
                            widgets::primary_button(ui, theme, "Connect")
                        })
                        .inner
                        .clicked();
                },
            );
            if connect {
                // A listed Strobe is asked for approval, never with a trust
                // file or a code left in the form.
                panel.controller.address = address;
                panel.controller.code.clear();
                panel.controller.use_trust = false;
                actions.push(Action::ConnectDesktop);
            }
        }
        let note = panel.controller.note();
        let mut row = Row::new(HOST).description(
            "For a Strobe that is not listed: its address, and the port if it is not 47812.",
        );
        if !note.is_empty() {
            row = row.note(&note, theme.text_muted);
        }
        group.row(row, |ui| {
            if widgets::button(ui, theme, None, "Connect").clicked() {
                actions.push(Action::ConnectDesktop);
            }
            let address = form::field(
                ui,
                theme,
                &mut panel.controller.address,
                "e.g. 192.168.1.20",
                FIELD,
            );
            if reveal {
                address.request_focus();
            }
        });
        group.toggle(
            Row::new(USE_TRUST).description(
                "For an unattended show machine: Connect by address proves a trust file instead \
                 of asking, and keeps trying again after a drop until you press Disconnect.",
            ),
            &mut panel.controller.use_trust,
        );
        if panel.controller.use_trust {
            group.row(
                Row::new("Trust file").description(
                    "The host must have been started with the same file (strobe --remote \
                     --allow-desktop-control --remote-trust FILE). Only the file's path is \
                     remembered, never its contents.",
                ),
                |ui| {
                    form::field(
                        ui,
                        theme,
                        &mut panel.controller.trust_path,
                        "Trust file, e.g. C:\\Users\\you\\venue.trust",
                        FIELD,
                    );
                },
            );
        }
    });
}

/// The listener's address and port, set before it starts: the last group,
/// since the system's choice is right almost always.
pub fn network(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>) {
    let running = panel.view.running;
    form::group(ui, theme, "Network", |group| {
        let mut row = Row::new("Address").enabled(!running).description(
            "An IP address of this computer. Empty uses its LAN address; 0.0.0.0 listens on \
             every interface. A public address is refused.",
        );
        if running {
            row = row.note(
                "Switch Let others connect off to change it.",
                theme.text_muted,
            );
        }
        group.row(row, |ui| {
            form::field(
                ui,
                theme,
                panel.address,
                "this computer's LAN address",
                FIELD,
            );
        });
        group.row(Row::new("Port").enabled(!running), |ui| {
            form::field(ui, theme, panel.port, "47812", FIELD / 2.0);
        });
    });
}
