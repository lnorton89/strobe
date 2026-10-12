// ============================================================================
// Module:       strobe_remote
// Description:  Versioned, transport-independent messages for LAN show control.
//
// Dependencies: serde, serde_json
// ============================================================================

//! Typed protocol shared by Strobe's LAN remote server and its clients.
//!
//! This crate deliberately does not open sockets, own app state, or depend on
//! the renderer. The network adapter must authenticate and validate a message,
//! then enqueue its command for the app's event-loop thread. It must never
//! call renderer or playlist objects directly.
//!
//! The protocol is intentionally allowlisted. There is no arbitrary command,
//! path, URL, script, or shader payload. Unknown message types and commands
//! fail deserialization instead of being ignored.
//!
//! The crate is the first implementation slice for the LAN remote described
//! in `docs/REMOTE_CONTROL_PWA_RESEARCH.md`; it is not by itself a running
//! server or an installable PWA.

use serde::{Deserialize, Serialize};

/// Current wire-protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Maximum request identifier length accepted by adapters.
pub const MAX_REQUEST_ID_LEN: usize = 64;

/// Maximum human-readable error length emitted to clients.
pub const MAX_ERROR_LEN: usize = 160;

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
    Ping,
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
    /// Register a tap-tempo event.
    TapTempo,
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
    /// Response to a ping.
    Pong,
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

    #[test]
    fn command_messages_round_trip_as_stable_snake_case() {
        let message = ClientMessage::Command {
            request_id: "phone-42".to_owned(),
            command: RemoteCommand::ToggleBlackout,
        };
        let json = serde_json::to_string(&message).unwrap_or_default();
        assert_eq!(
            json,
            r#"{"type":"command","request_id":"phone-42","command":"toggle_blackout"}"#
        );
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&json),
            Ok(message)
        );
    }

    #[test]
    fn unknown_message_types_and_fields_are_rejected() {
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"exec","code":"x"}"#).is_err());
        assert!(serde_json::from_str::<ClientMessage>(
            r#"{"type":"ping","unexpected":true}"#
        )
        .is_err());
    }

    #[test]
    fn unknown_commands_are_rejected() {
        assert!(serde_json::from_str::<RemoteCommand>(r#""run_shell""#).is_err());
    }

    #[test]
    fn request_ids_are_bounded_printable_ascii() {
        assert!(valid_request_id("phone-42"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("contains space"));
        assert!(!valid_request_id("line\nbreak"));
        assert!(!valid_request_id(&"x".repeat(MAX_REQUEST_ID_LEN + 1)));
    }

    #[test]
    fn state_messages_round_trip() {
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
        let json = serde_json::to_string(&message).unwrap_or_default();
        assert_eq!(serde_json::from_str::<ServerMessage>(&json), Ok(message));
    }
}
