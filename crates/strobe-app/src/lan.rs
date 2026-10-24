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
//! The address a phone opens is worked out when the listener starts and
//! again every [`URL_REFRESH`] (D341). For a listener bound to every
//! interface, working it out opens a UDP socket to ask the operating system
//! which address it would use, so it is not done on every control-window
//! frame; a changed lease still shows up within the period.
//!
//! A listener on the address the system chose (the address field empty)
//! checks every [`ADDRESS_CHECK`] that the address is still this computer's,
//! and moves to the new one when it is not (D461): the self-assigned
//! `169.254.x.x` a venue PC takes when it boots before the router, which
//! Windows drops once DHCP answers, or a lease that changed. A listener on
//! an address the operator typed stays there.
//!
//! The paired-client list (a small JSON file) is read from the data
//! directory once, the first time the control window or a start asks for it,
//! and revoking works with the listener off. The control window reads it
//! without waiting (D462): a phone authenticating holds it while its new
//! token reaches the disk, and a frame shows the list it had instead.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_remote::bridge::{self, AppEnd, Incoming, Snapshot};
use strobe_remote::clients::{ClientInfo, Clients};
use strobe_remote::pairing::Request;
use strobe_remote::server::{Server, ServerConfig, net};
use strobe_remote::trust::TrustSecret;
use strobe_remote::{AudioStatus, RemoteCommand};

/// Commands taken off the bridge in one frame. A phone sends a few a
/// second; the rest wait for the next frame (and the bridge is bounded).
pub const MAX_PER_FRAME: usize = 8;

/// How often the state is published: often enough that a press shows on
/// the phone at once, rarely enough to cost nothing.
pub const PUBLISH_PERIOD: Duration = Duration::from_millis(100);

/// How long the address shown to the operator is trusted before the system
/// is asked again. A DHCP change takes at most this long to show; asking
/// every control frame cost a socket about thirty times a second (D341).
const URL_REFRESH: Duration = Duration::from_secs(10);

/// How often a listener on the address the system chose checks that it is
/// still this computer's (D461). One socket bind each time, on the frame
/// thread; a moved address is followed within this long.
const ADDRESS_CHECK: Duration = Duration::from_secs(10);

/// Commands the bridge holds before the listener answers `Busy`.
const QUEUE: usize = 16;

/// How long a listener that was asked for at launch waits before trying to
/// start again, when the first try failed (no network yet, say).
const LAUNCH_RETRY: Duration = Duration::from_secs(5);

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
    /// The host was started with a trust file (D288): a controller that
    /// proves it holds it is paired for full desktop control, without an
    /// approval, while full desktop access is on.
    pub venue: bool,
    /// Other Strobes on the LAN can find this one (D468): the listener is
    /// answering discovery.
    pub advertising: bool,
    /// The name other Strobes list this one under.
    pub name: String,
}

/// The address a phone opens, remembered between asks.
#[derive(Debug, Default)]
struct UrlCache {
    text: String,
    /// When `text` was worked out; `None` when it has not been.
    at: Option<Instant>,
}

impl UrlCache {
    /// Remembers `text` as worked out at `now`.
    fn set(&mut self, now: Instant, text: String) {
        self.text = text;
        self.at = Some(now);
    }

    /// Forgets the address, so the next [`Self::refresh`] asks.
    fn clear(&mut self) {
        self.text.clear();
        self.at = None;
    }

    /// Asks `ask` for the address if there is none or it is [`URL_REFRESH`]
    /// old at `now`, and says whether that changed the text.
    fn refresh(&mut self, now: Instant, ask: impl FnOnce() -> String) -> bool {
        let fresh = self
            .at
            .is_some_and(|at| now.saturating_duration_since(at) < URL_REFRESH);
        if fresh {
            return false;
        }
        let text = ask();
        let changed = text != self.text;
        self.set(now, text);
        changed
    }
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
    /// The venue trust file's secret, read once at launch (D288). There is
    /// deliberately no way to set it from the UI.
    trust: Option<TrustSecret>,
    /// `--allow-desktop-control`: switch full desktop access on the first
    /// time the listener starts. Consumed then, so stopping and starting
    /// the remote by hand later does not turn it back on.
    desktop_at_start: bool,
    /// A launch-time start that failed is tried again at this time.
    retry_at: Option<Instant>,
    note: String,
    failed: bool,
    /// The address shown while the listener runs (D341).
    url: UrlCache,
    /// The running listener's address is the one the system chose (the
    /// field was empty), so it follows the address when it moves (D461).
    follows_lan: bool,
    /// When that address was last checked.
    address_checked: Option<Instant>,
    /// The paired list as last read without waiting (D462).
    listed: Vec<ClientInfo>,
    published: Option<Instant>,
    snapshot: Snapshot,
    /// The name other Strobes list this one under (D468), made once.
    name: Option<String>,
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
            trust: None,
            desktop_at_start: false,
            retry_at: None,
            note: "Off".to_string(),
            failed: false,
            url: UrlCache::default(),
            follows_lan: false,
            address_checked: None,
            listed: Vec::new(),
            published: None,
            snapshot: Snapshot::default(),
            name: None,
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

    /// Venue mode (D288): the trust file's secret, and whether full desktop
    /// access comes on with the first start (`--allow-desktop-control`).
    #[must_use]
    pub fn venue(mut self, trust: Option<TrustSecret>, allow_desktop: bool) -> Self {
        self.trust = trust;
        self.desktop_at_start = allow_desktop;
        self
    }

    /// The start `--remote` asks for. If it fails, for instance because the
    /// network is not up yet on a machine that has just booted, it is tried
    /// again every few seconds until it works or the operator stops or
    /// starts the remote by hand. Only the first failure is reported.
    pub fn start_at_launch(&mut self, now: Instant) {
        self.start();
        if !self.running() {
            self.retry_at = Some(now + LAUNCH_RETRY);
            log::warn!("remote: will keep trying to start every {LAUNCH_RETRY:?}");
        }
    }

    /// Once a frame: retries a launch-time start that failed, and moves a
    /// listener whose system-chosen address went away (D461).
    pub fn maintain(&mut self, now: Instant) {
        self.follow_address(now, net::is_local_address, net::lan_address);
        let Some(at) = self.retry_at else {
            return;
        };
        if self.server.is_some() {
            self.retry_at = None;
        } else if now >= at {
            self.start_inner(false);
            self.retry_at = (!self.running()).then_some(now + LAUNCH_RETRY);
        }
    }

    /// At most every [`ADDRESS_CHECK`]: if the listener is on the address
    /// the system chose and `assigned` says that address is no longer this
    /// computer's, listens on the one `current` now gives, on the same port.
    /// Before D461 the listener stayed on the dead address until Strobe was
    /// restarted: a venue PC that booted before its router listened on
    /// `169.254.x.x` all night and no laptop or phone could reach it.
    ///
    /// Moving drops every connection (a laptop in venue mode proves its way
    /// back in) and ends full desktop grants, as stopping does (D275), but
    /// keeps the full-desktop switch and the trust file: the host's
    /// launch-time choices stand. No address now (the network is down) is
    /// not a move; the listener stays and the next check looks again.
    fn follow_address(
        &mut self,
        now: Instant,
        assigned: impl FnOnce(IpAddr) -> bool,
        current: impl FnOnce() -> Option<IpAddr>,
    ) {
        if !self.follows_lan {
            return;
        }
        let Some(server) = &self.server else {
            return;
        };
        if self
            .address_checked
            .is_some_and(|at| now.saturating_duration_since(at) < ADDRESS_CHECK)
        {
            return;
        }
        self.address_checked = Some(now);
        let bound = server.addr();
        if assigned(bound.ip()) {
            return;
        }
        let Some(ip) = current().filter(|ip| *ip != bound.ip()) else {
            return;
        };
        log::warn!(
            "remote: {} is no longer this computer's address (DHCP gave it a new one, or the \
             network came up after the remote started); listening on {ip} instead",
            bound.ip()
        );
        // Whatever was in flight is answered Unavailable; the old listener
        // is stopped off the frame thread, since stopping waits up to a tick
        // for its accept thread, and the new one is on another address.
        self.app_end = None;
        if let Some(clients) = &self.clients {
            clients.revoke_desktop();
        }
        if let Some(old) = self.server.take() {
            // If no thread can be made, the closure is dropped with the
            // server in it, and dropping stops it here instead.
            let _ = std::thread::Builder::new()
                .name("strobe-remote-stop".to_string())
                .spawn(move || old.stop());
        }
        self.url.clear();
        self.start_at(Ok((ip, bound.port())), true);
        if !self.running() {
            self.retry_at = Some(now + LAUNCH_RETRY);
        }
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
        self.retry_at = None;
        self.start_inner(true);
    }

    fn start_inner(&mut self, report: bool) {
        if self.server.is_some() {
            return;
        }
        let target = self.target();
        self.start_at(target, report);
    }

    /// Starts the listener on `target`, the address and port the fields
    /// gave (or, moving, the system's new address, D461).
    fn start_at(&mut self, target: Result<(IpAddr, u16), String>, report: bool) {
        if self.server.is_some() {
            return;
        }
        let clients = self.clients();
        let started = target.and_then(|(ip, port)| {
            let (remote, app_end) = bridge::bridge(QUEUE);
            app_end
                .desktop
                .enable(self.desktop_enabled || self.desktop_at_start);
            let config = ServerConfig {
                proxied: self.proxied.clone(),
                trust: self.trust.clone(),
                // Found by other Strobes on the LAN while it listens (D468).
                advertise: Some(crate::trusted::computer_name()),
                ..ServerConfig::new(ip, port)
            };
            Server::start(config, remote, clients)
                .map(|server| (server, app_end))
                .map_err(|error| error.to_string())
        });
        match started {
            Ok((server, app_end)) => {
                let url = server.url();
                self.note = match &self.proxied {
                    Some(name) => format!("On: https://{name} (through your proxy)"),
                    None => format!("On: {url}"),
                };
                self.url.set(Instant::now(), url);
                self.failed = false;
                self.follows_lan = self.address.trim().is_empty() && self.proxied.is_none();
                self.address_checked = Some(Instant::now());
                if std::mem::take(&mut self.desktop_at_start) {
                    self.desktop_enabled = true;
                    log::info!("remote: full desktop access is on (--allow-desktop-control)");
                }
                self.server = Some(server);
                self.app_end = Some(app_end);
                self.published = None;
            }
            Err(error) => {
                self.note = error.clone();
                self.failed = true;
                if report {
                    failure::report(
                        Failure::new(Subsystem::Io, "remote")
                            .message(error)
                            .action("the LAN remote stays off"),
                    );
                } else {
                    log::debug!("remote: still not started: {error}");
                }
            }
        }
    }

    /// Stops the listener: every phone is dropped at once and the port is
    /// freed. Stopping a stopped one does nothing.
    pub fn stop(&mut self) {
        self.retry_at = None;
        self.desktop_at_start = false;
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
        self.url.clear();
        self.follows_lan = false;
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
        self.view_at(Instant::now())
    }

    /// [`Self::view`] at `now`: the address is asked for again only when it
    /// is [`URL_REFRESH`] old, and the "On: ..." line follows it if it moved.
    pub fn view_at(&mut self, now: Instant) -> LanView {
        // A connection saving the list holds it across a disk flush; the
        // frame shows the list it had rather than wait for that (D462).
        if let Some(fresh) = self.clients().try_list() {
            self.listed = fresh;
        }
        let clients = self.listed.clone();
        let Some(server) = &self.server else {
            return LanView {
                note: self.note.clone(),
                failed: self.failed,
                clients: clients.into_iter().map(|c| (c, false)).collect(),
                venue: self.trust.is_some(),
                name: self.name(),
                ..LanView::default()
            };
        };
        if self.url.refresh(now, || server.url()) && self.proxied.is_none() {
            self.note = format!("On: {}", self.url.text);
        }
        let connections = server.connections();
        let connected = |id: &str| {
            connections
                .iter()
                .any(|c| c.client_id.as_deref() == Some(id))
        };
        LanView {
            running: true,
            url: Some(self.url.text.clone()),
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
            venue: self.trust.is_some(),
            advertising: server.advertising(),
            name: self.name(),
        }
    }

    /// The name other Strobes list this one under, made once.
    fn name(&mut self) -> String {
        self.name
            .get_or_insert_with(|| {
                strobe_remote::discovery::display_name(&crate::trusted::computer_name())
            })
            .clone()
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
    fn the_address_is_asked_for_once_per_period_not_once_per_frame() {
        let start = Instant::now();
        let asked = std::cell::Cell::new(0_u32);
        let mut cache = UrlCache::default();
        let ask = |text: &str| {
            asked.set(asked.get() + 1);
            text.to_string()
        };
        assert!(
            cache.refresh(start, || ask("http://192.168.1.20:47812/")),
            "the first ask fills it"
        );
        for frame in 1..=300_u64 {
            let now = start + Duration::from_millis(frame * 33);
            assert!(
                !cache.refresh(now, || ask("http://192.168.1.99:47812/")),
                "frame {frame}: inside the period nothing is asked"
            );
        }
        assert_eq!(asked.get(), 1, "ten seconds of frames made one ask");
        assert_eq!(
            cache.text, "http://192.168.1.20:47812/",
            "and kept its answer"
        );

        let later = start + URL_REFRESH;
        assert!(
            cache.refresh(later, || ask("http://192.168.1.99:47812/")),
            "a new lease shows up once the period is over"
        );
        assert_eq!(cache.text, "http://192.168.1.99:47812/", "the new address");
        assert!(
            !cache.refresh(later + URL_REFRESH, || ask("http://192.168.1.99:47812/")),
            "asked again, the same answer is no change"
        );
        assert_eq!(asked.get(), 3, "one ask per period");

        cache.clear();
        assert!(
            cache.refresh(later + URL_REFRESH, || ask("http://10.0.0.5:47812/")),
            "a cleared cache asks at once"
        );
        assert_eq!(asked.get(), 4, "after a stop and start");
    }

    #[test]
    fn a_running_remote_does_not_ask_for_its_address_every_frame() -> Result<()> {
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false);
        lan.address = "127.0.0.1".into();
        lan.port = "47815".into();
        lan.start();
        if !lan.running() {
            // The port is taken on this machine; nothing else to check.
            return Ok(());
        }
        let primed = lan.url.at;
        assert!(primed.is_some(), "starting worked the address out");
        let url = lan.view().url;
        assert_eq!(
            url.as_deref(),
            Some("http://127.0.0.1:47815/"),
            "the view carries it"
        );
        let soon = Instant::now() + URL_REFRESH / 2;
        assert_eq!(lan.view_at(soon).url, url, "same address");
        assert_eq!(lan.url.at, primed, "a view inside the period asked nothing");
        let overdue = Instant::now() + URL_REFRESH * 2;
        assert_eq!(
            lan.view_at(overdue).url,
            url,
            "same address after a refresh"
        );
        assert_eq!(
            lan.url.at,
            Some(overdue),
            "a view past the period asked again"
        );
        lan.stop();
        assert!(lan.url.at.is_none(), "stopping forgets the address");
        Ok(())
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

    #[test]
    fn launch_flags_turn_desktop_access_on_once_and_the_view_says_venue() -> Result<()> {
        let secret = TrustSecret::generate()?;
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false).venue(Some(secret), true);
        assert!(lan.view().venue, "a trust file is loaded");
        assert!(
            !lan.desktop_enabled,
            "nothing is on before the listener starts"
        );
        lan.address = "127.0.0.1".into();
        lan.port = "47814".into();
        lan.start_at_launch(Instant::now());
        if !lan.running() {
            // The port is taken on this machine; nothing else to check.
            return Ok(());
        }
        assert!(lan.desktop_enabled, "--allow-desktop-control took effect");
        assert!(
            lan.desktop().is_some_and(|desktop| desktop.enabled()),
            "and the bridge agrees"
        );
        lan.stop();
        assert!(!lan.desktop_enabled, "stopping switches it off");
        lan.start();
        if lan.running() {
            assert!(
                !lan.desktop_enabled,
                "starting again by hand does not turn it back on"
            );
        }
        lan.stop();
        let mut plain = Lan::new(std::env::temp_dir(), None, 0, false);
        assert!(!plain.view().venue, "no trust file, no venue");
        Ok(())
    }

    #[test]
    fn a_listener_whose_chosen_address_went_away_moves_to_the_new_one() -> Result<()> {
        // A venue PC that boots before its router takes a self-assigned
        // 169.254.x.x, the remote binds it, and Windows drops it when DHCP
        // answers. The stand-ins here are two loopback addresses.
        let old: IpAddr = "127.0.0.2".parse()?;
        let new: IpAddr = "127.0.0.1".parse()?;
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false)
            .venue(Some(TrustSecret::generate()?), true);
        lan.port = "47818".into();
        // The address field is empty: the system chose `old`.
        lan.start_at(Ok((old, 47818)), true);
        if !lan.running() {
            // 127.0.0.2 is not usable here, or the port is taken.
            return Ok(());
        }
        let bound = |lan: &Lan| lan.server.as_ref().map(|server| server.addr().ip());
        let t0 = Instant::now();
        let asked = std::cell::Cell::new(0_u32);
        let ask = |ip: Option<IpAddr>| {
            asked.set(asked.get() + 1);
            ip
        };
        lan.follow_address(t0, |_| false, || ask(Some(new)));
        assert_eq!(asked.get(), 0, "checked when it started; not again so soon");
        let t1 = t0 + ADDRESS_CHECK;
        lan.follow_address(t1, |_| true, || ask(Some(new)));
        assert_eq!(
            bound(&lan),
            Some(old),
            "still this computer's address: it stays, whatever the route says"
        );
        let t2 = t1 + ADDRESS_CHECK;
        lan.follow_address(t2, |_| false, || ask(None));
        assert_eq!(
            bound(&lan),
            Some(old),
            "gone, but nothing new yet: it waits"
        );
        let t3 = t2 + ADDRESS_CHECK;
        lan.follow_address(t3, |ip| ip != old, || ask(Some(new)));
        assert_eq!(bound(&lan), Some(new), "gone and replaced: it follows");
        let view = lan.view_at(t3);
        assert_eq!(
            view.url.as_deref(),
            Some("http://127.0.0.1:47818/"),
            "Settings shows the new address: {view:?}"
        );
        assert!(
            lan.desktop_enabled && lan.desktop().is_some_and(|desktop| desktop.enabled()),
            "the launch-time full-desktop switch is kept"
        );
        assert!(
            lan.server
                .as_ref()
                .is_some_and(strobe_remote::server::Server::trust_configured),
            "and so is the trust file"
        );
        lan.stop();

        let mut typed = Lan::new(std::env::temp_dir(), None, 0, false);
        typed.address = "127.0.0.2".into();
        typed.port = "47819".into();
        typed.start();
        if typed.running() {
            typed.follow_address(t3 + ADDRESS_CHECK, |_| false, || Some(new));
            assert_eq!(
                bound(&typed),
                Some(old),
                "an address the operator typed is never changed"
            );
            typed.stop();
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn the_control_window_does_not_wait_for_the_client_list_to_reach_the_disk() -> Result<()> {
        // A phone authenticating rotates its token and saves the list, with
        // the list held, through `sync_all`; on a venue PC's slow storage
        // that is tens of milliseconds or more. The control window shows the
        // list every frame, and the output is drawn on the same thread. A
        // FIFO where the save's temporary file goes stands in for a disk
        // that does not answer: opening it to write waits for a reader.
        let dir = std::env::temp_dir().join(format!("strobe-lan-slow-disk-{}", std::process::id()));
        // Best effort: a leftover from an earlier run.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let fifo = dir.join("remote-clients.json.tmp");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            // No mkfifo here; nothing to check.
            std::fs::remove_dir_all(&dir)?;
            return Ok(());
        }
        let mut lan = Lan::new(dir.clone(), None, 0, true);
        let clients = lan.clients();
        assert!(lan.view_at(Instant::now()).clients.is_empty(), "none yet");
        let saving = std::thread::spawn(move || clients.pair("iPhone at 192.168.1.23").is_ok());
        // Let the save reach the disk that does not answer.
        std::thread::sleep(Duration::from_millis(200));
        let release = {
            let fifo = fifo.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                // The "disk" answers: whatever the save wrote is read and
                // dropped, and the save then fails on the FIFO, as one on a
                // real disk would not; only the wait matters here.
                let _ = std::fs::read(&fifo);
            })
        };
        let started = Instant::now();
        let view = lan.view_at(Instant::now());
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_secs(1),
            "the frame waited {waited:?} for another thread's save"
        );
        assert!(
            view.clients.is_empty(),
            "it shows the list it had: {view:?}"
        );
        release
            .join()
            .map_err(|_| anyhow::anyhow!("the release thread panicked"))?;
        // The save's outcome is the FIFO's, not a real disk's.
        let _ = saving.join();
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_launch_time_start_that_fails_is_retried_until_it_works() -> Result<()> {
        let mut lan = Lan::new(std::env::temp_dir(), None, 0, false);
        lan.address = "not an address".into();
        lan.port = "47815".into();
        let t0 = Instant::now();
        lan.start_at_launch(t0);
        assert!(!lan.running() && lan.view().failed, "the first try failed");
        lan.maintain(t0 + Duration::from_secs(1));
        assert!(!lan.running(), "not yet time");
        lan.address = "127.0.0.1".into();
        lan.maintain(t0 + LAUNCH_RETRY + Duration::from_secs(1));
        if !lan.running() {
            // The port is taken on this machine; still scheduled, not lost.
            assert!(lan.retry_at.is_some(), "it keeps trying");
            return Ok(());
        }
        assert!(lan.retry_at.is_none(), "and stops once it works");
        lan.stop();
        // A hand start or stop ends the retrying.
        let mut again = Lan::new(std::env::temp_dir(), None, 0, false);
        again.address = "not an address".into();
        again.start_at_launch(t0);
        again.stop();
        again.address = "127.0.0.1".into();
        again.maintain(t0 + LAUNCH_RETRY * 2);
        assert!(!again.running(), "a stopped remote is not restarted");
        Ok(())
    }
}
