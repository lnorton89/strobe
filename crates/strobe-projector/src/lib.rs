// ============================================================================
// Module:       strobe_projector (library crate root)
// Description:  Projector power over the network: the PJLink and NTCONTROL
//               clients, the safe power sequence, discovery, the DHCP
//               helpers, the config file, the audit log, and the
//               password-protected web page with the service that hosts it.
//
// Dependencies: serde, serde_json, thiserror, log, md-5, sha2, chrono;
//               strobe-diag (paths)
// ============================================================================

//! Switching the venue's projectors on and off from the show PC (D502).
//!
//! Strobe drives the picture; this crate drives the projectors' power. It
//! started as a stdlib-only Python tool (PR #45) and keeps that tool's
//! behaviour, command for command:
//!
//! - [`pjlink`] speaks PJLink Class 1 on TCP 4352, one connection per
//!   command, with the MD5 password prefix when the unit asks for it.
//! - [`ntcontrol`] speaks Panasonic's own protocol on TCP 1024, used only
//!   when PJLink is unusable on a unit configured with it as a fallback.
//! - [`unit`] is the safe power sequence: never a power command while a
//!   unit warms up or cools down, nothing sent when it is already in the
//!   requested state, ERR3 and refused connections retried with backoff
//!   for up to 90 s, and `--wait`'s polling (through the PT-DZ6700's lamp
//!   strike).
//! - [`config`] is `projectors.json`, which lives in Strobe's data
//!   directory and never in the repository; [`audit`] is the command log.
//! - [`discover`] and [`dhcp`] find units on the projector LAN, including
//!   one stuck asking for a DHCP lease, and can answer it.
//! - [`server`] is the web page and JSON API, a small HTTP/1.1 subset like
//!   `strobe-remote`'s, behind a password (D516). [`bind`] chooses the address (the
//!   Tailscale or Wi-Fi IPv4, never every interface), and [`service`]
//!   keeps the listener on it, moving when the address changes.
//!
//! Strobe's Settings > Projectors page uses all of this in-process; the
//! `strobe-projector` binary is the same library as a CLI and as the
//! headless service a Scheduled Task runs when Strobe is closed.

pub mod audit;
pub mod bind;
pub mod config;
pub mod dhcp;
pub mod discover;
pub mod error;
pub mod format;
pub mod ntcontrol;
pub mod pjlink;
pub mod server;
pub mod service;
pub mod unit;
pub mod wire;

pub use config::Config;
pub use error::Error;
pub use unit::{Power, Unit};
