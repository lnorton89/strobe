// ============================================================================
// Module:       strobe_app::lan
// Description:  The LAN remote as the app holds it: the listener, the app end
//               of its bridge, the paired clients, which commands it may run
//               now, and the state it publishes. Off until asked for.
//
// Dependencies: strobe-remote (Server, bridge, Clients); strobe-diag
//               (failure reports)
// ============================================================================

//! The app's side of the LAN remote (L-02 to L-06, D250 to D254).
//!
//! [`Lan`] owns the listener while it runs and the app end of its bridge.
//! Nothing binds until the operator switches it on in Settings or starts
//! Strobe with `--remote`; the switch is never saved, so a launch never opens
//! a socket on its own (D254).
//!
//! Once a frame, before the scene advances, the app takes up to
//! [`MAX_PER_FRAME`] commands off the bridge (never waiting), asks
//! [`refusal`] whether each can run *now*, runs it through the same
//! `Action` a key or an OSC message would, and answers it. At
//! [`PUBLISH_PERIOD`] it publishes the state a phone shows; the bridge sends
//! nothing when nothing changed.
//!
//! The paired-client list (a small JSON file) is read from the data
//! directory once, the first time the control window or a start asks for it,
//! and revoking works with the listener off.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_remote::bridge::{self, AppEnd, Incoming, Snapshot};
use strobe_remote::clients::{ClientInfo, Clients};
use strobe_remote::pairing::Request;
use strobe_remote::server::{Server, ServerConfig, net};
use strobe_remote::{AudioStatus, RemoteCommand};

/// Commands taken off the bridge in one frame. A phone sends a few a
/// second; the rest wait for the next frame (and the bridge is bounded).
pub const MAX_PER_FRAME: usize = 8;

/// How often the state is published: often enough that a press shows on
/// the phone at once, rarely enough to cost nothing.
pub const PUBLISH_PERIOD: Duration = Duration::from_millis(100);

/// Commands the bridge holds before the listener answers `Busy`.
const QUEUE: usize = 16;

/// The longest scene name sent to a phone, in characters.
const MAX_NAME: usize = 120;

/// What the app is doing, as far as deciding a remote command goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Situation {
    /// A playlist is loaded.
    pub playlist: bool,
    /// It has compiled and is playing.
    pub ready: bool,
    /// Next and previous have somewhere to go.
    pub can_step: bool,
}

/// Why `command` cannot run now, or `None` when it can. The reasons are
/// what the phone shows, so they say what to do.
pub fn refusal(command: RemoteCommand, now: Situation) -> Option<&'static str> {
    let preparing = now.playlist && !now.ready;
    match command {
        RemoteCommand::NextScene | RemoteCommand::PrevScene => {
            if preparing {
                Some("the playlist is still preparing")
            } else if !now.playlist && !now.can_step {
                Some("there is no other scene to step to")
            } else {
                None
            }
        }
        RemoteCommand::ToggleHold
        | RemoteCommand::RestartCue
        | RemoteCommand::ExtendCue
        | RemoteCommand::SkipSection => {
            if !now.playlist {
                Some("needs a playlist; load one on the desktop")
            } else if preparing {
                Some("the playlist is still preparing")
            } else {
                None
            }
        }
        RemoteCommand::ToggleBlackout
        | RemoteCommand::BlackoutOn
        | RemoteCommand::BlackoutOff
        | RemoteCommand::TapTempo => None,
    }
}

/// The audio readout in the protocol's words.
pub fn audio_status(live: bool, can_capture: bool) -> AudioStatus {
    match (live, can_capture) {
        (true, _) => AudioStatus::Active,
        (false, true) => AudioStatus::Missing,
        (false, false) => AudioStatus::Synthetic,
    }
}

/// What Settings and the title bar show, made once per control-window frame.
#[derive(Clone, Debug, Default)]
pub struct LanView {
    /// The listener is running.
    pub running: bool,
    /// The address a phone opens, while running.
    pub url: Option<String>,
    /// A warning about the bind (every interface, loopback).
    pub warning: Option<String>,
    /// One line: where it listens, or why it does not.
    pub note: String,
    /// The last start failed.
    pub failed: bool,
    /// The pairing code and the seconds it has left.
    pub code: Option<(String, u64)>,
    /// Phones waiting for Approve or Deny.
    pub requests: Vec<Request>,
    /// Paired clients, and whether each is connected now.
    pub clients: Vec<(ClientInfo, bool)>,
    /// Paired clients connected now: connections that authenticated. A
    /// socket that was refused, or is still waiting for approval, is not
    /// counted (it is no one's connection to the show).
    pub connected: usize,
    /// A paired desktop holds the input lease: someone elsewhere is
    /// operating this window (D275). The title bar says so.
    pub desktop_controlled: bool,
}

/// The LAN remote.
#[derive(Debug)]
pub struct Lan {
    /// Explicit full desktop access for this run; never saved or inherited
    /// from the phone listener's state.
    pub desktop_enabled: bool,
    server: Option<Server>,
    app_end: Option<AppEnd>,
    clients: Option<Clients>,
    data_dir: PathBuf,
    /// Write the client list to disk (not in screenshot mode).
    persist: bool,
    /// The address field: empty for this computer's LAN address.
    pub address: String,
    /// The port field.
    pub port: String,
    /// The operator's HTTPS reverse proxy's public name (D258), if any.
    proxied: Option<String>,
    note: String,
    failed: bool,
    published: Option<Instant>,
    snapshot: Snapshot,
}

impl Lan {
    /// The remote, off, with the command line's address and port.
    pub fn new(data_dir: PathBuf, bind: Option<IpAddr>, port: u16, persist: bool) -> Self {
        Self {
            desktop_enabled: false,
            server: None,
            app_end: None,
            clients: None,
            data_dir,
            persist,
            address: bind.map(|ip| ip.to_string()).unwrap_or_default(),
            port: port.to_string(),
            proxied: None,
            note: "Off".to_string(),
            failed: false,
            published: None,
            snapshot: Snapshot::default(),
        }
    }

    /// Also answers the operator's HTTPS proxy at `name` (D258); with no
    /// address given, the listener then binds 127.0.0.1, where the proxy
    /// reaches it.
    #[must_use]
    pub fn behind_proxy(mut self, name: Option<String>) -> Self {
        self.proxied = name;
        self
    }

    /// Whether the listener is running.
    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    /// The paired clients, read from disk the first time.
    fn clients(&mut self) -> Clients {
        if let Some(clients) = &self.clients {
            return clients.clone();
        }
        let clients = if self.persist {
            let (clients, problem) = Clients::load(&self.data_dir);
            if let Some(problem) = problem {
                failure::report(
                    Failure::new(Subsystem::Io, "remote clients")
                        .path(self.data_dir.join(strobe_remote::clients::CLIENTS_FILE))
                        .message(problem)
                        .action("paired phones must pair again"),
                );
            }
            clients
        } else {
            Clients::in_memory()
        };
        self.clients = Some(clients.clone());
        clients
    }

    /// The address and port the fields ask for.
    fn target(&self) -> Result<(IpAddr, u16), String> {
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| format!("\"{}\" is not a port (1 to 65535)", self.port.trim()))?;
        if port == 0 {
            return Err("the port must be 1 to 65535".to_string());
        }
        let text = self.address.trim();
        let ip = if text.is_empty() && self.proxied.is_some() {
            IpAddr::from([127, 0, 0, 1])
        } else if text.is_empty() {
            net::lan_address().ok_or_else(|| {
                "this computer has no LAN address right now (is Wi-Fi or Ethernet connected?); \
                 type one in the address field"
                    .to_string()
            })?
        } else {
            text.parse::<IpAddr>().map_err(|_| {
                format!("\"{text}\" is not an IP address (a name is not allowed, D252)")
            })?
        };
        Ok((ip, port))
    }

    /// Starts the listener. Starting a running one does nothing. A failure
    /// is shown in Settings and reported, and the remote stays off.
    pub fn start(&mut self) {
        if self.server.is_some() {
            return;
        }
        let target = self.target();
        let clients = self.clients();
        let started = target.and_then(|(ip, port)| {
            let (remote, app_end) = bridge::bridge(QUEUE);
            app_end.desktop.enable(self.desktop_enabled);
            let config = ServerConfig {
                proxied: self.proxied.clone(),
                ..ServerConfig::new(ip, port)
            };
            Server::start(config, remote, clients)
                .map(|server| (server, app_end))
                .map_err(|error| error.to_string())
        });
        match started {
            Ok((server, app_end)) => {
                self.note = match &self.proxied {
                    Some(name) => format!("On: https://{name} (through your proxy)"),
                    None => format!("On: {}", server.url()),
                };
                self.failed = false;
                self.server = Some(server);
                self.app_end = Some(app_end);
                self.published = None;
            }
            Err(error) => {
                self.note = error.clone();
                self.failed = true;
                failure::report(
                    Failure::new(Subsystem::Io, "remote")
                        .message(error)
                        .action("the LAN remote stays off"),
                );
            }
        }
    }

    /// Stops the listener: every phone is dropped at once and the port is
    /// freed. Stopping a stopped one does nothing.
    pub fn stop(&mut self) {
        if let Some(app_end) = &self.app_end {
            app_end.desktop.enable(false);
        }
        self.desktop_enabled = false;
        // A full desktop grant lasts as long as the switch it was approved
        // under (D275); the server is stopped below, so the list is enough.
        if let Some(clients) = &self.clients {
            clients.revoke_desktop();
        }
        // The bridge first, so a command in flight is answered Unavailable.
        self.app_end = None;
        if let Some(server) = self.server.take() {
            server.stop();
        }
        if !self.failed {
            self.note = "Off".to_string();
        }
    }

    /// The next command waiting, if any. Never blocks.
    pub fn next_command(&self) -> Option<Incoming> {
        self.app_end.as_ref().and_then(AppEnd::next)
    }

    /// Switch full desktop access without changing phone command access.
    /// Switching it off also revokes every desktop pairing and drops its
    /// connection, so the next session must be approved again (D275).
    pub fn set_desktop(&mut self, enabled: bool) {
        self.desktop_enabled = enabled && self.running();
        if let Some(app_end) = &self.app_end {
            app_end.desktop.enable(self.desktop_enabled);
        }
        if !self.desktop_enabled {
            match &self.server {
                Some(server) => {
                    server.revoke_desktop();
                }
                None => {
                    if let Some(clients) = &self.clients {
                        clients.revoke_desktop();
                    }
                }
            }
        }
    }

    /// Whether a desktop controller holds the input lease right now.
    pub fn desktop_controlled(&self) -> bool {
        self.app_end
            .as_ref()
            .is_some_and(|app_end| app_end.desktop.active())
    }

    /// Desktop bridge while the listener is running.
    pub fn desktop(&self) -> Option<strobe_remote::desktop::Desktop> {
        self.app_end.as_ref().map(|app_end| app_end.desktop.clone())
    }

    /// Whether it is time to publish the state.
    pub fn publish_due(&mut self, now: Instant) -> bool {
        if self.app_end.is_none() {
            return false;
        }
        if self
            .published
            .is_some_and(|at| now.saturating_duration_since(at) < PUBLISH_PERIOD)
        {
            return false;
        }
        self.published = Some(now);
        true
    }

    /// Publishes the state, rebuilding the snapshot's text only when it
    /// changed.
    #[expect(
        clippy::too_many_arguments,
        reason = "one argument per field of the protocol's state message"
    )]
    pub fn publish(
        &mut self,
        current: Option<&str>,
        next: Option<&str>,
        remaining: Option<f32>,
        held: bool,
        blackout: bool,
        output_visible: bool,
        audio: AudioStatus,
    ) {
        let Some(app_end) = self.app_end.as_ref() else {
            return;
        };
        let snapshot = &mut self.snapshot;
        set_name(&mut snapshot.current, current);
        set_name(&mut snapshot.next, next);
        // Whole seconds, rounded up: the phone shows m:ss, and a value that
        // moves every frame would be a message every publish.
        snapshot.remaining_ms = remaining.map(|seconds| (seconds.max(0.0).ceil() as u64) * 1000);
        snapshot.held = held;
        snapshot.blackout = blackout;
        snapshot.output_visible = output_visible;
        snapshot.audio_status = audio;
        app_end.publish(snapshot);
    }

    /// Opens pairing with a fresh code.
    pub fn open_pairing(&mut self) {
        let Some(server) = &self.server else {
            return;
        };
        if let Err(error) = server.open_pairing() {
            self.note = format!("Could not make a pairing code: {error}");
        }
    }

    /// Closes pairing.
    pub fn close_pairing(&self) {
        if let Some(server) = &self.server {
            server.close_pairing();
        }
    }

    /// Approves or denies a waiting phone.
    pub fn decide(&self, id: u64, approve: bool) {
        if let Some(server) = &self.server {
            server.decide(id, approve);
        }
    }

    /// Revokes one client, closing its connection if it has one.
    pub fn revoke(&mut self, id: &str) {
        match &self.server {
            Some(server) => {
                server.revoke(id);
            }
            None => {
                self.clients().revoke(id);
                log::info!("remote: client {id} revoked");
            }
        }
    }

    /// Revokes every client.
    pub fn revoke_all(&mut self) {
        match &self.server {
            Some(server) => {
                server.revoke_all();
            }
            None => {
                let count = self.clients().revoke_all();
                log::info!("remote: all {count} clients revoked");
            }
        }
    }

    /// What the UI shows this frame.
    pub fn view(&mut self) -> LanView {
        let clients = self.clients().list();
        let Some(server) = &self.server else {
            return LanView {
                note: self.note.clone(),
                failed: self.failed,
                clients: clients.into_iter().map(|c| (c, false)).collect(),
                ..LanView::default()
            };
        };
        let connections = server.connections();
        let connected = |id: &str| {
            connections
                .iter()
                .any(|c| c.client_id.as_deref() == Some(id))
        };
        LanView {
            running: true,
            url: Some(server.url()),
            warning: server.warning().map(str::to_string),
            note: self.note.clone(),
            failed: false,
            code: server
                .pairing_code()
                .map(|(code, left)| (code, left.as_secs())),
            requests: server.pairing_requests(),
            clients: clients
                .into_iter()
                .map(|c| {
                    let on = connected(&c.id);
                    (c, on)
                })
                .collect(),
            connected: authenticated(&connections),
            desktop_controlled: self.desktop_controlled(),
        }
    }
}

/// How many of `connections` are paired clients that authenticated.
fn authenticated(connections: &[strobe_remote::server::ConnectionInfo]) -> usize {
    connections
        .iter()
        .filter(|connection| connection.is_authenticated())
        .count()
}

/// Sets `slot` to `name` (cut to [`MAX_NAME`]), allocating only when it
/// changed.
fn set_name(slot: &mut Option<String>, name: Option<&str>) {
    match (slot.as_deref(), name) {
        (Some(have), Some(want)) if have == want => {}
        (None, None) => {}
        (Some(_) | None, Some(want)) => {
            *slot = Some(want.chars().take(MAX_NAME).collect());
        }
        (Some(_), None) => *slot = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    const LIBRARY: Situation = Situation {
        playlist: false,
        ready: false,
        can_step: true,
    };

    #[test]
    fn only_authenticated_clients_count_as_connected() {
        use strobe_remote::server::ConnectionInfo;
        let socket = |port: u16, client_id: Option<&str>| ConnectionInfo {
            peer: std::net::SocketAddr::from(([192, 168, 1, 30], port)),
            client_id: client_id.map(str::to_string),
        };
        assert_eq!(authenticated(&[]), 0, "nothing open");
        assert_eq!(
            authenticated(&[socket(50001, None)]),
            0,
            "a refused or unapproved socket is not a connected client"
        );
        assert_eq!(
            authenticated(&[
                socket(50001, None),
                socket(50002, Some("c00000001")),
                socket(50003, None),
                socket(50004, Some("c00000002")),
            ]),
            2,
            "two clients among four sockets"
        );
    }

    #[test]
    fn nothing_listens_until_the_remote_is_started() {
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false);
        assert!(!lan.running(), "off");
        assert!(lan.next_command().is_none(), "no bridge");
        assert!(!lan.publish_due(Instant::now()), "nothing to publish to");
        let view = lan.view();
        assert!(!view.running && view.url.is_none(), "{view:?}");
        lan.stop();
        lan.stop();
        assert_eq!(lan.view().note, "Off", "stopping twice is harmless");
    }

    #[test]
    fn playlist_commands_need_a_ready_playlist() {
        let preparing = Situation {
            playlist: true,
            ready: false,
            can_step: false,
        };
        let playing = Situation {
            playlist: true,
            ready: true,
            can_step: true,
        };
        for command in [
            RemoteCommand::ToggleHold,
            RemoteCommand::RestartCue,
            RemoteCommand::ExtendCue,
            RemoteCommand::SkipSection,
        ] {
            assert!(
                refusal(command, LIBRARY).is_some(),
                "{command:?} without a playlist"
            );
            assert!(
                refusal(command, preparing).is_some(),
                "{command:?} while preparing"
            );
            assert_eq!(refusal(command, playing), None, "{command:?} while playing");
        }
        assert_eq!(
            refusal(RemoteCommand::NextScene, LIBRARY),
            None,
            "library order"
        );
        assert!(
            refusal(
                RemoteCommand::PrevScene,
                Situation {
                    can_step: false,
                    ..LIBRARY
                }
            )
            .is_some(),
            "a library of one"
        );
        for command in [
            RemoteCommand::BlackoutOn,
            RemoteCommand::BlackoutOff,
            RemoteCommand::ToggleBlackout,
            RemoteCommand::TapTempo,
        ] {
            assert_eq!(refusal(command, preparing), None, "{command:?} always runs");
        }
    }

    #[test]
    fn every_remote_command_is_decided() {
        for command in RemoteCommand::ALL {
            let _ = refusal(command, LIBRARY);
        }
        assert_eq!(audio_status(true, true), AudioStatus::Active, "live");
        assert_eq!(audio_status(false, true), AudioStatus::Missing, "no input");
        assert_eq!(
            audio_status(false, false),
            AudioStatus::Synthetic,
            "synthetic"
        );
    }

    #[test]
    fn a_name_is_cut_and_only_replaced_when_it_changes() {
        let mut slot = None;
        set_name(&mut slot, Some(&"a".repeat(500)));
        assert_eq!(slot.as_deref().map(str::len), Some(MAX_NAME), "cut");
        set_name(&mut slot, Some("Aurora"));
        let before = slot.as_ref().map(|s| s.as_ptr());
        set_name(&mut slot, Some("Aurora"));
        assert_eq!(
            slot.as_ref().map(|s| s.as_ptr()),
            before,
            "unchanged, no new string"
        );
        set_name(&mut slot, None);
        assert_eq!(slot, None, "cleared");
    }

    #[test]
    fn a_bad_address_or_port_is_refused_with_a_reason() -> Result<()> {
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false);
        lan.address = "phone.local".into();
        lan.port = "47812".into();
        lan.start();
        let view = lan.view();
        assert!(!view.running && view.failed, "refused");
        assert!(view.note.contains("not an IP address"), "{}", view.note);
        lan.address = "127.0.0.1".into();
        lan.port = "70000".into();
        lan.start();
        assert!(lan.view().note.contains("not a port"), "port");
        lan.address = "8.8.8.8".into();
        lan.port = "47812".into();
        lan.start();
        assert!(lan.view().note.contains("public"), "a public address");
        Ok(())
    }

    #[test]
    fn the_remote_starts_publishes_and_stops() -> Result<()> {
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false);
        lan.address = "127.0.0.1".into();
        lan.port = "0".into();
        lan.start();
        assert!(!lan.running(), "port 0 is refused in the field");
        lan.port = "47813".into();
        lan.start();
        if !lan.running() {
            // The port is taken on this machine; nothing else to check.
            return Ok(());
        }
        let view = lan.view();
        assert!(
            view.url.as_deref() == Some("http://127.0.0.1:47813/"),
            "{view:?}"
        );
        assert!(
            view.warning.is_some(),
            "loopback says a phone cannot reach it"
        );
        assert!(lan.publish_due(Instant::now()), "the first publish is due");
        lan.publish(
            Some("Aurora"),
            None,
            Some(1.2),
            false,
            true,
            true,
            AudioStatus::Active,
        );
        assert_eq!(lan.snapshot.remaining_ms, Some(2000), "whole seconds, up");
        lan.open_pairing();
        assert!(lan.view().code.is_some(), "a code shows");
        lan.stop();
        assert!(!lan.running(), "stopped");
        assert!(lan.view().code.is_none(), "and the code is gone");
        Ok(())
    }
}
