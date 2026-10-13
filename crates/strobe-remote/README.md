# strobe-remote

Strobe's LAN show remote: a phone or tablet on the same network runs the show
(next/previous, hold, restart, extend, skip section, blackout, tap tempo)
while the laptop drives the projector. The design and threat model are in
[`REMOTE_CONTROL_PWA_RESEARCH.md`](../../docs/REMOTE_CONTROL_PWA_RESEARCH.md);
the decisions are D232, D236 and D250 to D254.

The crate holds the protocol, the bridge to the app thread, pairing and
paired clients, and the listener. The listener is **opt-in**: nothing binds
until the app calls `Server::start`. It never touches the renderer, the
playlist or the UI: a command goes onto `bridge::Remote`, the app takes it off
`bridge::AppEnd` on its own thread and runs it through the same command path a
key, a pad or an OSC message uses (D172, D232, D254).

It is a **plain-HTTP remote for trusted networks**, not an installable PWA:
that waits on a TLS decision (backlog L-07).

## Role in the workspace

- **Depends on:** `serde`, `serde_json`, `getrandom` (codes and tokens),
  `sha2` (stored token digests, the page's CSP hashes), `sha1` (the WebSocket
  handshake only, D250), `log`.
- **Depended on by:** `strobe-app` (`lan.rs`, Settings' Remote section, the
  title bar's "Remote on" and Stop).
- **Never depends on:** another Strobe crate, `strobe-gfx`, a window or egui.

## Main types / entry points

| Item | What |
|---|---|
| `ClientMessage` | `hello`, `pair`, `authenticate`, `command`, `ping`. Internally tagged by `type`, `deny_unknown_fields`: an unknown type or field fails to parse rather than being ignored. |
| `RemoteCommand` | The allowlist: no scene loading, files, scripts, shaders or settings. |
| `ServerMessage` | `hello`, `paired`, `authenticated`, a revisioned `state` snapshot, `command_result`, `error`, `pong`. A `paired` after `authenticated` is a token rotation (D251). |
| `negotiate`, `decode_client`, `ServerMessage::error` | Version 1 to `PROTOCOL_VERSION`, a bounded and validated decode (`MAX_MESSAGE_BYTES` 4096, `MAX_SECRET_LEN` 128), errors cut to `MAX_ERROR_LEN` (D236). |
| `bridge::bridge(capacity)` | `(Remote, AppEnd)`: a bounded command queue with a reply per request, and a revisioned `Snapshot` (D232). |
| `server::Server` | `start(ServerConfig, Remote, Clients)`, `stop`, `url`, `warning`, `open_pairing`, `pairing_code`, `pairing_requests`, `decide`, `connections`, `revoke`, `revoke_all`. `Limits` holds every bound (D250, D252). |
| `server::net` | `scope` (loopback, private, link-local, every interface, public), `lan_address`, `host_matches`. |
| `pairing::Pairing` | The six-digit code, its attempt limit and expiry, and requests waiting for Approve or Deny (D251). |
| `clients::Clients` | Paired clients in `remote-clients.json`: token digests, rotation, revocation (D251). |
| `secret::Token` | A 256-bit token that prints as `***` and is not `Serialize`. |

## The listener

```text
GET /    the controller page (src/server/controller.html, embedded)
GET /ws  WebSocket, protocol v1
```

Host must be the bound IP literal and port, Origin exactly the page's own;
frames are bounded before they are read; clients are rate-limited, timed out
when idle or unpaired, and dropped when they stop reading. There is no
outbound queue. `tests/server.rs` drives all of it over real sockets,
adversarial cases included.

## Notes

- `Ping` and `Pong` are empty struct variants, not unit variants:
  `deny_unknown_fields` does not reach a unit variant of an internally tagged
  enum, so a unit `Ping` accepted `{"type":"ping", <anything>}`.
- Tokens and pairing codes travel only in message bodies, never URLs or logs.
- The HTTP head reader is separate from the Shadertoy listener's on purpose
  (D250): this one reads with a deadline and hands the rest to the WebSocket.
