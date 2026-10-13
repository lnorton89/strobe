# strobe-remote

The wire protocol for Strobe's planned LAN show remote: a phone or tablet on
the same network running the show (next/previous, hold, restart, extend, skip
section, blackout, tap tempo) while the laptop drives the projector. The
design, threat model and remaining work are in
[`REMOTE_CONTROL_PWA_RESEARCH.md`](../../docs/REMOTE_CONTROL_PWA_RESEARCH.md)
(ROADMAP item 7b).

This crate is **protocol and bridge only**. It opens no socket, owns no app
state and never touches the renderer. A future network adapter authenticates
and validates each message, then hands the command to `bridge::Remote`; the
app takes it off `bridge::AppEnd` on its own thread and runs it through the
same command path a key, a pad or an OSC message uses (D172, D232).

## Role in the workspace

- **Depends on:** `serde`, `serde_json`.
- **Depended on by:** `strobe-app`'s tests only, which keep the command list
  aligned with the app's command ids. The listener, pairing and token
  storage, and the controller page are still open
  ([`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md), epic L).
- **Never depends on:** `strobe-gfx`, `strobe-app`, a window, or a socket.

## Main types / entry points

| Item | What |
|---|---|
| `ClientMessage` | `hello`, `pair`, `authenticate`, `command`, `ping`. Internally tagged by `type`, `deny_unknown_fields`: an unknown type or field fails to parse rather than being ignored. |
| `RemoteCommand` | The allowlist: no scene loading, files, scripts, shaders or settings. |
| `ServerMessage` | `hello`, `paired`, `authenticated`, a revisioned `state` snapshot, `command_result`, `error`, `pong`. |
| `ErrorCode`, `AudioStatus` | Stable codes and short states for a small screen. |
| `RemoteCommand::ALL`, `RemoteCommand::id` | Every command and its wire name, which is the app's `CommandId` id. |
| `bridge::bridge(capacity)` | `(Remote, AppEnd)`: a bounded command queue with a reply per request (`Pending::wait`), and a revisioned `Snapshot`. A full queue is `Busy` at once; a gone app is `Unavailable`. |
| `negotiate`, `decode_client`, `ServerMessage::error` | The adapter's side of every message: the session version (1 to `PROTOCOL_VERSION`), a bounded and validated decode (`MAX_MESSAGE_BYTES` 4096, `MAX_SECRET_LEN` 128) that answers another version's `hello` with `version_mismatch`, and errors cut to `MAX_ERROR_LEN`. |
| `valid_request_id` | Bounded printable-ASCII request ids. |
| `PROTOCOL_VERSION` | `1`. |

## Notes

- `Ping` and `Pong` are empty struct variants, not unit variants:
  `deny_unknown_fields` does not reach a unit variant of an internally tagged
  enum, so a unit `Ping` accepted `{"type":"ping", <anything>}`.
- Tokens and pairing codes travel only in message bodies, never URLs or logs.
