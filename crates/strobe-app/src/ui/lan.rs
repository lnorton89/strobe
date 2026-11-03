// ============================================================================
// Module:       strobe_app::ui::lan
// Description:  Settings' Remote page: let others connect (the LAN remote,
//               found by other Strobes, each request approved or denied),
//               the paired devices and what tells them apart, the Strobes
//               on the network to control, and the address and port.
//
// Dependencies: egui; super::{form, model, theme, widgets}; crate::{icon,
//               lan, palette}; strobe-remote (discovery, clients); std::time
// ============================================================================

//! The LAN remote (L-03, L-06, D254) and controlling another Strobe (D275),
//! in `super::form`'s grammar, streamlined (D468).
//!
//! One switch lets others connect. While it is on, other Strobes on the
//! network list this one by name, and anyone asking to connect is put in
//! front of the operator as a banner at the top of the page with Approve
//! and Deny. Nobody types a code (D468, D469): a desktop asks from its own
//! list, and a phone opens the address shown and asks under a name it is
//! given.
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
use std::time::{SystemTime, UNIX_EPOCH};
use strobe_remote::clients::{Access, ClientInfo};

// Row titles, also their entries in Settings' search.

/// The switch that lets others connect.
pub const REMOTE: &str = "Let others connect";
/// The phone row: the address a phone opens to ask.
pub const PAIR: &str = "Phone address";
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
            phones(group, theme, panel);
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

/// The phone rows, while the remote listens: the address a phone opens to
/// ask (D469), and any warning about the bind.
fn phones(group: &mut form::Group<'_>, theme: &Palette, panel: &LanPanel<'_>) {
    let view = panel.view;
    let url = view.url.as_deref().unwrap_or_default();
    let connected = format!(
        "Open it in a phone's browser and give the phone a name; approve it here when it \
         asks. {} paired device{} connected.",
        view.connected,
        if view.connected == 1 { "" } else { "s" }
    );
    group.row(
        Row::new(PAIR)
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        for (client, connected) in &view.clients {
            let words = DeviceWords::of(client, *connected, now);
            let (icon, word, colour) = if *connected {
                (Icon::Check, "Connected", theme.success)
            } else {
                (Icon::Info, "Not connected", theme.text_muted)
            };
            let row = Row::new(&words.title)
                .status(icon, word, colour)
                .description(&words.what)
                .note(&words.when, theme.text_muted);
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

/// What a paired device's row says, enough to tell it from the others and
/// decide whether to revoke it (D500).
#[derive(Debug, PartialEq, Eq)]
struct DeviceWords {
    /// The name it asked under (a desktop's is its computer's name), or
    /// its platform when it gave none.
    title: String,
    /// What kind of client it is and the address it last came from.
    what: String,
    /// When it paired and when it was last seen.
    when: String,
}

impl DeviceWords {
    /// `client`'s words at `now`, in seconds since 1970.
    fn of(client: &ClientInfo, connected: bool, now: u64) -> Self {
        let platform = client.platform.as_deref();
        let title = client
            .name
            .as_deref()
            .or(platform)
            .unwrap_or(&client.label)
            .to_string();
        let kind = match client.access {
            Access::Phone => match platform.filter(|_| client.name.is_some()) {
                Some(platform) => format!("Phone remote on {platform}"),
                None => "Phone remote".to_string(),
            },
            Access::Desktop => "Strobe desktop, full control".to_string(),
            Access::Trusted => "Strobe desktop, trusted by the venue file".to_string(),
        };
        let address = client
            .address
            .map_or_else(|| "address unknown".to_string(), |a| a.to_string());
        let seen = if connected {
            "now".to_string()
        } else {
            ago(client.last_seen, now)
        };
        Self {
            title,
            what: format!("{kind} · {address}"),
            when: format!("Paired {} · last seen {seen}", ago(client.paired_at, now)),
        }
    }
}

/// How long before `now` the moment `then` was, in words: `5 minutes ago`.
/// Both are seconds since 1970; a moment after `now` (the clock was set
/// back) is `just now`.
fn ago(then: u64, now: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let plural =
        |count: u64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    match now.saturating_sub(then) {
        elapsed if elapsed < MINUTE => "just now".to_string(),
        elapsed if elapsed < HOUR => plural(elapsed / MINUTE, "minute"),
        elapsed if elapsed < DAY => plural(elapsed / HOUR, "hour"),
        elapsed if elapsed < 2 * DAY => "yesterday".to_string(),
        elapsed if elapsed < 60 * DAY => plural(elapsed / DAY, "day"),
        elapsed if elapsed < 730 * DAY => plural(elapsed / (30 * DAY), "month"),
        elapsed => plural(elapsed / (365 * DAY), "year"),
    }
}

/// "Control another Strobe": the Strobes found on the network, each with
/// Connect, then an address typed by hand and a venue trust file (D275,
/// D468).
///
/// It is its own group, with the command palette's "Control another
/// Strobe..." entry to open it: nobody looking to control a second laptop
/// would think to look under being controlled. `reveal` scrolls to the
/// address field, lights it and focuses it.
pub fn control(
    ui: &mut Ui,
    theme: &Palette,
    panel: &mut LanPanel<'_>,
    picking: bool,
    actions: &mut Vec<Action>,
) {
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
                // Proves the trust file when one is chosen, and asks for
                // approval otherwise (D509).
                panel.controller.choose_found(found.addr);
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
                "For an unattended show machine: Connect proves a trust file instead of asking, \
                 and keeps trying again after a drop until you press Disconnect.",
            ),
            &mut panel.controller.use_trust,
        );
        if panel.controller.use_trust {
            group.wide_row(
                Row::new("Trust file").description(
                    "The host must have been started with the same file (strobe --remote \
                     --allow-desktop-control --remote-trust FILE). Only the file's path is \
                     remembered, never its contents.",
                ),
                |ui| {
                    if widgets::browse_button(ui, theme, picking) {
                        actions.push(Action::Browse(crate::picker::Purpose::TrustFile));
                    }
                    let width = ui.available_size_before_wrap().x;
                    form::field(
                        ui,
                        theme,
                        &mut panel.controller.trust_path,
                        "Trust file, e.g. C:\\Users\\you\\venue.trust",
                        width,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    /// 2026-10-27 12:00 UTC.
    const NOW: u64 = 1_793_102_400;

    fn client(name: Option<&str>, platform: Option<&str>, access: Access) -> ClientInfo {
        ClientInfo {
            id: "c00000001".into(),
            label: "Windows at 10.1.10.124".into(),
            name: name.map(str::to_owned),
            platform: platform.map(str::to_owned),
            address: Some(IpAddr::from([10, 1, 10, 124])),
            access,
            paired_at: NOW - 3 * 86_400,
            last_seen: NOW - 5 * 60,
        }
    }

    #[test]
    fn a_row_names_the_device_its_kind_address_and_times() {
        assert_eq!(
            DeviceWords::of(
                &client(Some("Ana's iPad"), Some("iPhone"), Access::Phone),
                false,
                NOW
            ),
            DeviceWords {
                title: "Ana's iPad".into(),
                what: "Phone remote on iPhone · 10.1.10.124".into(),
                when: "Paired 3 days ago · last seen 5 minutes ago".into(),
            },
            "a phone that gave a name"
        );
        assert_eq!(
            DeviceWords::of(&client(None, Some("Windows"), Access::Phone), true, NOW),
            DeviceWords {
                title: "Windows".into(),
                what: "Phone remote · 10.1.10.124".into(),
                when: "Paired 3 days ago · last seen now".into(),
            },
            "the owner's row from before names were kept, connected"
        );
        let desktop = DeviceWords::of(
            &client(
                Some("FOH-LAPTOP-1a2b3c4d"),
                Some("Strobe desktop"),
                Access::Desktop,
            ),
            false,
            NOW,
        );
        assert_eq!(
            (desktop.title.as_str(), desktop.what.as_str()),
            (
                "FOH-LAPTOP-1a2b3c4d",
                "Strobe desktop, full control · 10.1.10.124"
            ),
            "a desktop by its computer's name"
        );
        let trusted = DeviceWords::of(&client(Some("venue"), None, Access::Trusted), false, NOW);
        assert!(
            trusted.what.starts_with("Strobe desktop, trusted"),
            "a trusted controller says so: {}",
            trusted.what
        );
        let mut bare = client(None, None, Access::Phone);
        bare.address = None;
        let bare = DeviceWords::of(&bare, false, NOW);
        assert_eq!(
            (bare.title.as_str(), bare.what.as_str()),
            ("Windows at 10.1.10.124", "Phone remote · address unknown"),
            "nothing known: the label it was approved under"
        );
    }

    #[test]
    fn times_read_as_how_long_ago() {
        for (elapsed, words) in [
            (0, "just now"),
            (59, "just now"),
            (60, "1 minute ago"),
            (3_599, "59 minutes ago"),
            (3_600, "1 hour ago"),
            (86_399, "23 hours ago"),
            (86_400, "yesterday"),
            (2 * 86_400, "2 days ago"),
            (90 * 86_400, "3 months ago"),
            (800 * 86_400, "2 years ago"),
        ] {
            assert_eq!(ago(NOW - elapsed, NOW), words, "{elapsed} s");
        }
        assert_eq!(ago(NOW + 100, NOW), "just now", "a clock set back");
    }
}
