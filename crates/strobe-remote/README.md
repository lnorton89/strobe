# strobe-remote

The wire protocol for Strobe's planned LAN show remote: a phone or tablet on
the same network running the show (next/previous, hold, restart, extend, skip
section, blackout, tap tempo) while the laptop drives the projector. The
design, threat model and remaining work are in
[`REMOTE_CONTROL_PWA_RESEARCH.md`](../../docs/REMOTE_CONTROL_PWA_RESEARCH.md)
(ROADMAP item 7b).

This crate is **protocol only**. It opens no socket, owns no app state and
never touches the renderer. A future network adapter authenticates and
validates each message, then puts the command on the app's bounded command
queue, the same path OSC uses (D172), so the app thread runs it.

## Role in the workspace

- **Depends on:** `serde`, `serde_json`.
- **Depended on by:** nothing yet. The listener, pairing and token storage,
  the app-thread bridge and the controller page are still open
  ([`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md), epic L).
- **Never depends on:** `strobe-gfx`, `strobe-app`, a window, or a socket.

## Main types / entry points

| Item | What |
|---|---|
| `ClientMessage` | `hello`, `pair`, `authenticate`, `command`, `ping`. Internally tagged by `type`, `deny_unknown_fields`: an unknown type or field fails to parse rather than being ignored. |
| `RemoteCommand` | The allowlist: no scene loading, files, scripts, shaders or settings. |
| `ServerMessage` | `hello`, `paired`, `authenticated`, a revisioned `state` snapshot, `command_result`, `error`, `pong`. |
| `ErrorCode`, `AudioStatus` | Stable codes and short states for a small screen. |
| `valid_request_id` | Bounded printable-ASCII request ids. |
| `PROTOCOL_VERSION` | `1`. |

## Notes

- `Ping` and `Pong` are empty struct variants, not unit variants:
  `deny_unknown_fields` does not reach a unit variant of an internally tagged
  enum, so a unit `Ping` accepted `{"type":"ping", <anything>}`.
- Tokens and pairing codes travel only in message bodies, never URLs or logs.
