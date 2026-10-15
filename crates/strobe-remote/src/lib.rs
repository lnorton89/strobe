// ============================================================================
// Module:       strobe_remote (library crate root)
// Description:  Strobe's LAN show remote: the versioned protocol, the
//               app-thread bridge, pairing and paired clients, and the
//               opt-in listener that serves the controller page.
//
// Dependencies: serde, serde_json, getrandom, sha2, sha1, hmac, log
// ============================================================================

//! Typed protocol shared by Strobe's LAN remote server and its clients, and
//! the server itself.
//!
//! - The **protocol** (this file): allowlisted, versioned messages. Phone
//!   commands never carry files, scripts or settings. The separately
//!   approved desktop extension carries UI input, including editor text,
//!   rather than direct filesystem or script RPC. Unknown message types
//!   and commands fail deserialization instead of being ignored (D236).
//! - The **bridge** ([`bridge`], D232): the bounded queue and revisioned
//!   snapshot where a network thread meets the app's event-loop thread. The
//!   listener never calls renderer or playlist objects; the app runs each
//!   command on its own thread through its ordinary command path.
//! - **Pairing** ([`pairing`]), **paired clients** ([`clients`]) and their
//!   **secrets** ([`secret`]) (D251).
//! - The **listener** ([`server`], D250, D252, D253): opt-in, bound to a
//!   chosen private address, serving one embedded page and one WebSocket on
//!   threads of its own.
//!
//! This is a plain-HTTP LAN remote for trusted networks. It is not an
//! installable PWA: that waits on a TLS decision (backlog L-07).

pub mod bridge;
pub mod client;
pub mod clients;
pub mod desktop;
pub mod pairing;
pub mod secret;
pub mod server;
pub mod trust;

use serde::{Deserialize, Serialize};

/// Current wire-protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Maximum request identifier length accepted by adapters.
pub const MAX_REQUEST_ID_LEN: usize = 64;

/// Maximum human-readable error length emitted to clients.
pub const MAX_ERROR_LEN: usize = 160;

/// The oldest client protocol version this server still speaks.
pub const MIN_PROTOCOL_VERSION: u16 = 1;

/// The largest client message [`decode_client`] reads, in bytes. Every
/// message is a few dozen bytes; anything near this is not a client.
pub const MAX_MESSAGE_BYTES: usize = 4096;

/// The longest pairing code or token accepted, in bytes.
pub const MAX_SECRET_LEN: usize = 128;

/// A client-to-server message. Adapters must enforce a small frame-size limit
/// before deserializing and must authenticate every command after pairing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    /// Start a protocol session and negotiate its version.
    Hello {
        /// Client's supported protocol version.
        protocol_version: u16,
    },
    /// Exchange the short-lived pairing code shown by the desktop app.
    Pair {
        /// One-time code; never log this field.
        code: String,
    },
    /// Pair for full desktop input, with a separately labelled host approval.
    PairDesktop {
        /// One-use code; host full desktop access must also be enabled.
        code: String,
    },
    /// Authenticate a previously paired client on a new connection.
    ///
    /// Adapters must compare this token in constant time where practical,
    /// never log it, and accept it only in the message body (never a URL).
    Authenticate {
        /// Opaque per-client bearer token.
        token: String,
    },
    /// Request one allowlisted show-control operation.
    Command {
        /// Caller-generated id used to correlate the result.
        request_id: String,
        /// The operation to execute.
        command: RemoteCommand,
    },
    /// Keep a connection alive without requesting any state change.
    ///
    /// An empty struct, not a unit variant: serde's `deny_unknown_fields`
    /// does not apply to a unit variant of an internally tagged enum, so
    /// `Ping` would accept `{"type":"ping","anything":...}`.
    Ping {},
    /// Request exclusive control of the desktop UI. Requires the host's
    /// separate full-desktop switch as well as ordinary pairing.
    DesktopOpen {},
    /// Ordered UI events, applied only by the exclusive desktop controller.
    DesktopInput {
        /// A bounded batch of pointer, keyboard and text events.
        events: Vec<desktop::Input>,
    },
}

/// Commands intentionally exposed to a paired phone or tablet.
///
/// Scene loading, filesystem access, arbitrary scripts, shader editing and
/// configuration changes are excluded from the remote-control surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommand {
    /// Load the next library scene when no playlist is active, or advance the
    /// playlist when one is active.
    NextScene,
    /// Load the previous library scene when no playlist is active, or move to
    /// the previous cue when supported by the active playlist.
    PrevScene,
    /// Toggle hold/resume on an active playlist.
    ToggleHold,
    /// Restart the current playlist cue.
    RestartCue,
    /// Add the standard 30 seconds to the current cue.
    ExtendCue,
    /// Skip to the next playlist section.
    SkipSection,
    /// Toggle output blackout immediately.
    ToggleBlackout,
    /// Black out the output; already black stays black. A phone that lost
    /// its connection cannot know the current state, so it sends this
    /// rather than a toggle that could bring the output back.
    BlackoutOn,
    /// Bring the output back; already showing stays showing.
    BlackoutOff,
    /// Register a tap-tempo event.
    TapTempo,
}

impl RemoteCommand {
    /// Every command a remote may send.
    pub const ALL: [Self; 10] = [
        Self::NextScene,
        Self::PrevScene,
        Self::ToggleHold,
        Self::RestartCue,
        Self::ExtendCue,
        Self::SkipSection,
        Self::ToggleBlackout,
        Self::BlackoutOn,
        Self::BlackoutOff,
        Self::TapTempo,
    ];

    /// The command's wire name, which is also the app's command id
    /// (`strobe_app`'s `CommandId::id`), so a remote command runs through
    /// the same path as a key, a pad or an OSC message.
    pub fn id(self) -> &'static str {
        match self {
            Self::NextScene => "next_scene",
            Self::PrevScene => "prev_scene",
            Self::ToggleHold => "toggle_hold",
            Self::RestartCue => "restart_cue",
            Self::ExtendCue => "extend_cue",
            Self::SkipSection => "skip_section",
            Self::ToggleBlackout => "toggle_blackout",
            Self::BlackoutOn => "blackout_on",
            Self::BlackoutOff => "blackout_off",
            Self::TapTempo => "tap_tempo",
        }
    }
}

/// A server-to-client message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerMessage {
    /// Server protocol and pairing state.
    Hello {
        /// Server's supported protocol version.
        protocol_version: u16,
        /// Whether this connection has been paired.
        paired: bool,
    },
    /// Pairing completed; the token must be delivered only over the protected
    /// pairing exchange and stored by the client as securely as the platform
    /// allows. Adapters must never put it in a URL or log.
    Paired {
        /// Opaque identifier used to revoke this client on the desktop.
        client_id: String,
        /// High-entropy bearer token. Send it only in an authenticated channel.
        token: String,
    },
    /// A previously paired client has authenticated successfully.
    Authenticated {
        /// Stable identifier for the paired client.
        client_id: String,
    },
    /// Complete current-state snapshot, sent on connect and resynchronization.
    State {
        /// Monotonically increasing revision for stale-update detection.
        revision: u64,
        /// Current scene or cue label, if one is active.
        current: Option<String>,
        /// Next scene or cue label, if known.
        next: Option<String>,
        /// Remaining cue duration in milliseconds; absent without a timed cue.
        remaining_ms: Option<u64>,
        /// Whether the active playlist is held.
        held: bool,
        /// Whether output is blacked out.
        blackout: bool,
        /// Whether the output window is visible.
        output_visible: bool,
        /// Short audio input status suitable for a small screen.
        audio_status: AudioStatus,
    },
    /// Acknowledgment for a command request.
    CommandResult {
        /// The request identifier supplied by the client.
        request_id: String,
        /// Whether the app accepted the command for execution.
        accepted: bool,
        /// Short reason when the command was rejected.
        reason: Option<String>,
    },
    /// A bounded, user-safe protocol error.
    Error {
        /// Stable machine-readable code.
        code: ErrorCode,
        /// Short human-readable explanation.
        message: String,
    },
    /// Response to a ping (an empty struct, for the reason on `Ping`).
    Pong {},
    /// The host's control window, JPEG encoded. Only the desktop lease
    /// holder receives these; phones retain their original protocol.
    DesktopFrame {
        /// Capture revision.
        revision: u64,
        /// Pixel dimensions of the captured host window.
        size: [u32; 2],
        /// Standard base64 JPEG data.
        jpeg: String,
    },
    /// Text the host UI explicitly copied or cut for the controller.
    DesktopClipboard {
        /// Selected Unicode text (at most 64 KiB).
        text: String,
    },
}

/// Audio state as displayed by the remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioStatus {
    /// A real input is active.
    Active,
    /// The app is running without live input.
    Missing,
    /// The app is using generated test audio.
    Synthetic,
    /// Audio state is not yet known.
    Unknown,
}

/// Stable error categories; implementation details stay in the desktop log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The client and server speak incompatible protocol versions.
    VersionMismatch,
    /// Pairing code is invalid or expired.
    PairingRejected,
    /// Client is not paired or its token was revoked.
    Unauthorized,
    /// The requested command is not valid in the current app state.
    InvalidState,
    /// Request identifier or message shape failed validation.
    InvalidRequest,
    /// The app's bounded command queue is full.
    Busy,
    /// Server is stopping or the remote has been disabled.
    Unavailable,
}

/// The version a session runs at for a client that said `client` in its
/// `hello`, or [`ErrorCode::VersionMismatch`] when this server does not
/// speak it. Versions are whole: there is no partial support.
pub fn negotiate(client: u16) -> Result<u16, ErrorCode> {
    if (MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&client) {
        Ok(client)
    } else {
        Err(ErrorCode::VersionMismatch)
    }
}

/// Decodes one client message, as an adapter must before acting on it:
/// at most [`MAX_MESSAGE_BYTES`], a known type with exactly its fields, a
/// valid request id, and secrets no longer than [`MAX_SECRET_LEN`]. A
/// `hello` from a client of another version that this version cannot parse
/// (a newer client with new fields) is [`ErrorCode::VersionMismatch`], so
/// the client learns why rather than seeing a malformed-request error.
/// Every other failure is [`ErrorCode::InvalidRequest`].
pub fn decode_client(text: &str) -> Result<ClientMessage, ErrorCode> {
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(ErrorCode::InvalidRequest);
    }
    let message = match serde_json::from_str::<ClientMessage>(text) {
        Ok(message) => message,
        Err(_) => return Err(other_version(text).unwrap_or(ErrorCode::InvalidRequest)),
    };
    let valid = match &message {
        ClientMessage::Hello { .. } | ClientMessage::Ping {} => true,
        ClientMessage::Pair { code } | ClientMessage::PairDesktop { code } => {
            !code.is_empty() && code.len() <= MAX_SECRET_LEN
        }
        ClientMessage::Authenticate { token } => !token.is_empty() && token.len() <= MAX_SECRET_LEN,
        ClientMessage::Command { request_id, .. } => valid_request_id(request_id),
        ClientMessage::DesktopOpen {} => true,
        ClientMessage::DesktopInput { events } => desktop::valid_inputs(events),
    };
    if valid {
        Ok(message)
    } else {
        Err(ErrorCode::InvalidRequest)
    }
}

/// `VersionMismatch` if `text` is a `hello` naming a version other than
/// this one's, however its other fields look.
fn other_version(text: &str) -> Option<ErrorCode> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let is_hello = value.get("type")?.as_str()? == "hello";
    let version = value.get("protocol_version")?.as_u64()?;
    (is_hello && u16::try_from(version).map_or(true, |v| negotiate(v).is_err()))
        .then_some(ErrorCode::VersionMismatch)
}

impl ServerMessage {
    /// An error for a client, its message cut to [`MAX_ERROR_LEN`] bytes on
    /// a character boundary.
    pub fn error(code: ErrorCode, message: &str) -> Self {
        let mut end = message.len().min(MAX_ERROR_LEN);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self::Error {
            code,
            message: message.get(..end).unwrap_or_default().to_owned(),
        }
    }
}

/// Validates an opaque request identifier before it is used in a response.
///
/// IDs are restricted to printable ASCII to keep logs and UI diagnostics
/// predictable. Empty IDs, control characters and oversized values fail.
pub fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_LEN
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn command_messages_round_trip_as_stable_snake_case() -> Result<()> {
        let message = ClientMessage::Command {
            request_id: "phone-42".to_owned(),
            command: RemoteCommand::ToggleBlackout,
        };
        let json = serde_json::to_string(&message)?;
        assert_eq!(
            json, r#"{"type":"command","request_id":"phone-42","command":"toggle_blackout"}"#,
            "the wire form is stable snake_case"
        );
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json)?,
            message,
            "and reads back as the same message"
        );
        Ok(())
    }

    #[test]
    fn unknown_message_types_and_fields_are_rejected() {
        assert!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"exec","code":"x"}"#).is_err(),
            "an unknown message type fails"
        );
        assert!(
            serde_json::from_str::<ClientMessage>(
                r#"{"type":"hello","protocol_version":1,"unexpected":true}"#
            )
            .is_err(),
            "an unknown field fails"
        );
        assert!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"ping","unexpected":true}"#).is_err(),
            "an unknown field on a message with no fields fails too"
        );
        assert!(
            matches!(
                serde_json::from_str::<ClientMessage>(r#"{"type":"ping"}"#),
                Ok(ClientMessage::Ping {})
            ),
            "a plain ping still parses"
        );
    }

    #[test]
    fn explicit_blackout_commands_have_stable_names() -> Result<()> {
        assert_eq!(
            serde_json::to_string(&RemoteCommand::BlackoutOn)?,
            r#""blackout_on""#,
            "on"
        );
        assert_eq!(
            serde_json::from_str::<RemoteCommand>(r#""blackout_off""#)?,
            RemoteCommand::BlackoutOff,
            "off"
        );
        Ok(())
    }

    #[test]
    fn every_command_id_is_its_wire_name() -> Result<()> {
        for command in RemoteCommand::ALL {
            assert_eq!(
                serde_json::to_string(&command)?,
                format!("\"{}\"", command.id()),
                "{command:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn unknown_commands_are_rejected() {
        assert!(
            serde_json::from_str::<RemoteCommand>(r#""run_shell""#).is_err(),
            "only allowlisted commands parse"
        );
    }

    #[test]
    fn request_ids_are_bounded_printable_ascii() {
        assert!(valid_request_id("phone-42"), "an ordinary id");
        assert!(!valid_request_id(""), "empty");
        assert!(!valid_request_id("contains space"), "a space");
        assert!(!valid_request_id("line\nbreak"), "a control character");
        assert!(
            !valid_request_id(&"x".repeat(MAX_REQUEST_ID_LEN + 1)),
            "too long"
        );
    }

    #[test]
    fn state_messages_round_trip() -> Result<()> {
        let message = ServerMessage::State {
            revision: 7,
            current: Some("Aurora".to_owned()),
            next: Some("Orbit".to_owned()),
            remaining_ms: Some(30_000),
            held: false,
            blackout: true,
            output_visible: true,
            audio_status: AudioStatus::Active,
        };
        let json = serde_json::to_string(&message)?;
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&json)?,
            message,
            "a state snapshot reads back unchanged"
        );
        Ok(())
    }

    /// Every message of protocol version 1 as a client or server sends it.
    /// A change that breaks one of these breaks shipped clients.
    const V1_CLIENT: [&str; 6] = [
        r#"{"type":"hello","protocol_version":1}"#,
        r#"{"type":"pair","code":"481-226"}"#,
        r#"{"type":"authenticate","token":"tok_abc"}"#,
        r#"{"type":"command","request_id":"r1","command":"blackout_on"}"#,
        r#"{"type":"command","request_id":"r2","command":"tap_tempo"}"#,
        r#"{"type":"ping"}"#,
    ];

    const V1_SERVER: [&str; 7] = [
        r#"{"type":"hello","protocol_version":1,"paired":false}"#,
        r#"{"type":"paired","client_id":"c1","token":"tok_abc"}"#,
        r#"{"type":"authenticated","client_id":"c1"}"#,
        r#"{"type":"state","revision":7,"current":"Aurora","next":null,"remaining_ms":null,"held":false,"blackout":true,"output_visible":true,"audio_status":"active"}"#,
        r#"{"type":"command_result","request_id":"r1","accepted":false,"reason":"needs a playlist"}"#,
        r#"{"type":"error","code":"version_mismatch","message":"speak version 1"}"#,
        r#"{"type":"pong"}"#,
    ];

    #[test]
    fn version_one_messages_still_read_and_write_byte_for_byte() -> Result<()> {
        for text in V1_CLIENT {
            let message = decode_client(text).map_err(|e| anyhow::anyhow!("{text}: {e:?}"))?;
            assert_eq!(serde_json::to_string(&message)?, text, "client {text}");
        }
        for text in V1_SERVER {
            let message: ServerMessage = serde_json::from_str(text)?;
            assert_eq!(serde_json::to_string(&message)?, text, "server {text}");
        }
        Ok(())
    }

    /// The desktop extension's messages (D275), frozen like version 1's.
    /// A phone never sends or receives one; these are the native
    /// controller's shapes.
    const DESKTOP_CLIENT: [&str; 4] = [
        r#"{"type":"pair_desktop","code":"481226"}"#,
        r#"{"type":"desktop_open"}"#,
        r#"{"type":"desktop_input","events":[{"kind":"move","x":0.25,"y":0.5},{"kind":"button","x":0.25,"y":0.5,"button":0,"down":true,"modifiers":{"alt":false,"ctrl":true,"shift":false,"mac_cmd":false}}]}"#,
        r#"{"type":"desktop_input","events":[{"kind":"key","key":"Tab","down":true,"modifiers":{"alt":false,"ctrl":false,"shift":false,"mac_cmd":false}},{"kind":"text","text":"x"},{"kind":"paste","text":"y"},{"kind":"scroll","x":0.0,"y":-24.0,"modifiers":{"alt":false,"ctrl":false,"shift":false,"mac_cmd":false}},{"kind":"reset"},{"kind":"copy"},{"kind":"cut"}]}"#,
    ];

    const DESKTOP_SERVER: [&str; 2] = [
        r#"{"type":"desktop_frame","revision":3,"size":[640,480],"jpeg":"/9j/"}"#,
        r#"{"type":"desktop_clipboard","text":"vec4"}"#,
    ];

    #[test]
    fn desktop_messages_read_and_write_byte_for_byte() -> Result<()> {
        for text in DESKTOP_CLIENT {
            let message = decode_client(text).map_err(|e| anyhow::anyhow!("{text}: {e:?}"))?;
            assert_eq!(serde_json::to_string(&message)?, text, "client {text}");
        }
        for text in DESKTOP_SERVER {
            let message: ServerMessage = serde_json::from_str(text)?;
            assert_eq!(serde_json::to_string(&message)?, text, "server {text}");
        }
        Ok(())
    }

    #[test]
    fn other_versions_are_told_so_and_bad_messages_are_refused() {
        assert_eq!(negotiate(1), Ok(1), "this version");
        assert_eq!(negotiate(0), Err(ErrorCode::VersionMismatch), "older");
        assert_eq!(negotiate(2), Err(ErrorCode::VersionMismatch), "newer");
        assert_eq!(
            decode_client(r#"{"type":"hello","protocol_version":2,"features":["x"]}"#).err(),
            Some(ErrorCode::VersionMismatch),
            "a newer hello with new fields learns why"
        );
        assert_eq!(
            decode_client(r#"{"type":"hello","protocol_version":70000}"#).err(),
            Some(ErrorCode::VersionMismatch),
            "a version past u16"
        );
        let long = format!(
            r#"{{"type":"pair","code":"{}"}}"#,
            "1".repeat(MAX_SECRET_LEN + 1)
        );
        let huge = format!(
            r#"{{"type":"ping","pad":"{}"}}"#,
            " ".repeat(MAX_MESSAGE_BYTES)
        );
        for bad in [
            r#"{"type":"command","request_id":"has space","command":"tap_tempo"}"#,
            r#"{"type":"command","request_id":"r","command":"run_shell"}"#,
            r#"{"type":"authenticate","token":""}"#,
            r#"{"type":"hello","protocol_version":1,"extra":1}"#,
            "not json",
            long.as_str(),
            huge.as_str(),
        ] {
            assert_eq!(
                decode_client(bad).err(),
                Some(ErrorCode::InvalidRequest),
                "{}",
                bad.get(..60).unwrap_or(bad)
            );
        }
    }

    #[test]
    fn error_messages_are_cut_on_a_character_boundary() -> Result<()> {
        let long = "é".repeat(MAX_ERROR_LEN);
        let ServerMessage::Error { message, .. } = ServerMessage::error(ErrorCode::Busy, &long)
        else {
            anyhow::bail!("not an error message");
        };
        assert!(message.len() <= MAX_ERROR_LEN, "bounded: {}", message.len());
        assert!(message.chars().all(|c| c == 'é'), "no split character");
        Ok(())
    }
}
