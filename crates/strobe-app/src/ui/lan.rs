// ============================================================================
// Module:       strobe_app::ui::lan
// Description:  Settings' Remote page: switch the LAN remote on and off, its
//               address, pairing with a code and Approve/Deny, the paired
//               devices with Revoke, and the form for controlling another
//               Strobe.
//
// Dependencies: egui; super::{form, model, theme, widgets}; crate::{icon,
//               lan, palette}
// ============================================================================

//! The LAN remote's settings (L-03, L-06, D254) and the form for controlling
//! another Strobe desktop (D275), in `super::form`'s grammar.
//!
//! The groups read a [`LanView`](crate::lan::LanView) the app made this
//! frame and hand back [`Action`]s; they never touch the listener. The
//! order is the order a person needs it in: whether it is on and where,
//! then pairing (a code to type on the phone, then the phone to approve),
//! then who is paired. Revoking works with the remote off. The title bar
//! carries the always-visible "Remote on" readout and its one-click Stop
//! (`super::chrome`), and clicking it opens this page.

use super::form::{self, Row};
use super::model::{Action, LanPanel};
use super::theme::{self, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use egui::Ui;

// Row titles, also their entries in Settings' search.

/// The remote's on/off row.
pub const REMOTE: &str = "LAN remote";
/// The pairing row.
pub const PAIR: &str = "Pair a device";
/// The full-desktop-control switch.
pub const DESKTOP: &str = "Allow full desktop control";
/// The row that revokes every paired device.
pub const REVOKE_ALL: &str = "Revoke all";
/// The connect form's host address.
pub const HOST: &str = "Host address";
/// The connect form's venue-mode switch.
pub const USE_TRUST: &str = "Use a trust file (venue mode)";
/// The connect form's button row.
pub const CONNECT: &str = "Connect";

/// The width of the address and path fields.
const FIELD: f32 = 230.0;

/// Draws the "This computer" and "Paired devices" groups.
pub fn show(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let view = panel.view;
    form::group(ui, theme, "This computer", |group| {
        let (icon, word, colour) = if view.running {
            (Icon::Check, "On", theme.success)
        } else {
            (Icon::Info, "Off", theme.text_muted)
        };
        let mut row = Row::new(REMOTE).status(icon, word, colour).description(
            "Run the show from a phone, tablet or another Strobe desktop on the same network. \
             Off until you switch it on, and never on by itself at launch. Plain HTTP: use it \
             only on a network you trust.",
        );
        if view.failed {
            row = row.note(&view.note, theme.danger);
        }
        group.row(row, |ui| {
            if view.running {
                if widgets::button(ui, theme, Some(Icon::Close), "Stop remote")
                    .on_hover_text("Stops listening and drops every phone at once.")
                    .clicked()
                {
                    actions.push(Action::SetLanRemote(false));
                }
            } else if widgets::primary_button(ui, theme, "Start remote").clicked() {
                actions.push(Action::SetLanRemote(true));
            }
        });
        if view.running {
            running(group, theme, panel, actions);
        } else {
            group.row(
                Row::new("Address").description(
                    "An IP address of this computer. Empty uses its LAN address; 0.0.0.0 \
                     listens on every interface. A public address is refused.",
                ),
                |ui| {
                    form::field(
                        ui,
                        theme,
                        panel.address,
                        "this computer's LAN address",
                        FIELD,
                    );
                },
            );
            group.row(Row::new("Port"), |ui| {
                form::field(ui, theme, panel.port, "47812", FIELD / 2.0);
            });
        }
        let mut desktop_enabled = panel.desktop_enabled;
        let mut row = Row::new(DESKTOP).enabled(view.running).description(
            "Grants desktops you approve the entire Strobe window, editor and file saves \
             included, but not pairing or network settings. One controller at a time. \
             Switching it off, or stopping the remote, ends every desktop pairing.",
        );
        if !view.running {
            row = row.note("Start the remote to change this.", theme.text_muted);
        } else if view.venue {
            row = row.note(
                "Venue mode: this Strobe was started with a trust file. While full desktop \
                 control is on, a controller that holds the same file gets full control with no \
                 approval here. Revoke ends its session and refuses that controller until Strobe \
                 restarts; Revoke all, or switching full desktop control off, closes the door.",
                theme.warning,
            );
        }
        if group.toggle(row, &mut desktop_enabled).changed() {
            actions.push(Action::SetDesktopAccess(desktop_enabled));
        }
    });
    paired(ui, theme, panel, actions);
}

/// The rows that only exist while the remote listens: its address, any
/// warning, pairing and the requests waiting for an answer.
fn running(
    group: &mut form::Group<'_>,
    theme: &Palette,
    panel: &LanPanel<'_>,
    actions: &mut Vec<Action>,
) {
    let view = panel.view;
    let url = view.url.as_deref().unwrap_or_default();
    let connected = format!(
        "{} paired device{} connected. Windows may ask whether Strobe may accept connections: \
         allow it on private networks only.",
        view.connected,
        if view.connected == 1 { "" } else { "s" }
    );
    // The address in the row's words, where it can wrap; the control
    // column holds only Copy.
    group.row(
        Row::new("Address")
            .status(Icon::Check, url, theme.success)
            .description(&connected),
        |ui| {
            if widgets::button(ui, theme, Some(Icon::Copy), "Copy")
                .on_hover_text("Open this address in the phone's browser. It holds no secret.")
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
                "Type this code on the phone. It expires in {}:{:02}.",
                left / 60,
                left % 60
            );
            group.row(Row::new(PAIR).description(&expires), |ui| {
                if widgets::button(ui, theme, None, "Close pairing").clicked() {
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
                    "Shows a six-digit code for two minutes. The phone types it, then you \
                     approve the phone here.",
                ),
                |ui| {
                    if widgets::button(ui, theme, None, "Pair a device").clicked() {
                        actions.push(Action::OpenRemotePairing);
                    }
                },
            );
        }
    }
    for request in &view.requests {
        group.block(|ui| {
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
        });
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
            let mut row = Row::new(&client.label);
            if *connected {
                row = row.status(Icon::Check, "connected", theme.success);
            }
            if trusted {
                row = row.description("Trusted: holds this host's venue trust file.");
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

/// The "Control another Strobe" group: the form that makes this desktop the
/// controller of another one's whole window (D275).
///
/// It is its own group, with the command palette's "Control another
/// Strobe..." entry to open it, because it had been the last paragraph of
/// Remote, which is about being controlled: nobody looking to control a
/// second laptop would think to look there. `reveal` scrolls to the address
/// field, lights it and focuses it.
pub fn control(ui: &mut Ui, theme: &Palette, panel: &mut LanPanel<'_>, actions: &mut Vec<Action>) {
    let reveal = panel.controller.take_reveal();
    if reveal {
        form::reveal(HOST);
    }
    form::group(ui, theme, "Control another Strobe", |group| {
        group.block(|ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Operate another computer's whole Strobe window from here. On the host: \
                         Settings > Remote, Start remote, switch on Allow full desktop control, \
                         then Pair a device to get a six-digit code. Type its address and that \
                         code below.",
                    )
                    .font(theme::description_font())
                    .color(theme::rgb(theme.text_muted)),
                )
                .wrap(),
            );
        });
        group.row(
            Row::new(HOST).description("The port is 47812 unless the host says otherwise."),
            |ui| {
                let address = form::field(
                    ui,
                    theme,
                    &mut panel.controller.address,
                    "Host address, e.g. 192.168.1.20",
                    FIELD,
                );
                if reveal {
                    address.request_focus();
                }
            },
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
        } else {
            group.row(
                Row::new("Pairing code").description(
                    "The six digits the host shows. Leave it blank to reconnect to a host you \
                     paired with earlier in this session.",
                ),
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut panel.controller.code)
                            .password(true)
                            .hint_text(
                                egui::RichText::new("Pairing code")
                                    .color(theme::rgb(theme.text_faint)),
                            )
                            .desired_width(FIELD / 2.0)
                            .margin(theme::margin_xy(theme::SPACE_SM, SPACE_XS)),
                    );
                },
            );
        }
        group.toggle(
            Row::new(USE_TRUST).description(
                "For an unattended show machine. Nobody needs to approve anything on the host, \
                 and if the connection drops, or the host restarts, this keeps trying again \
                 until you press Disconnect.",
            ),
            &mut panel.controller.use_trust,
        );
        let note = panel.controller.note();
        let mut row = Row::new(CONNECT);
        if !note.is_empty() {
            row = row.description(&note);
        }
        group.row(row, |ui| {
            if widgets::primary_button(ui, theme, "Connect to desktop").clicked() {
                actions.push(Action::ConnectDesktop);
            }
        });
    });
}
