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

**Desktop-to-desktop control (D275):** the same listener also supports a
native Strobe controller that mirrors the complete host control window,
including scene editing and file operations. `pair_desktop` requires the
host's explicit full-desktop switch and a separately labelled approval;
ordinary phone tokens and old stored records never gain this permission.
One connection leases input at a time. Stop, revoke, overflow and app loss
release the lease and reset held input. The app supplies JPEG captures at
at most 5 Hz; the network bridge keeps only the latest frame. Text copied
by a host widget is returned explicitly, without reading its OS clipboard.
See [the user guide](../../docs/USER_GUIDE.md#control-another-strobe-desktop).

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
| `client::Connection` | Native worker transport: private IP address, bounded handshake/frame reader, random client masks and socket timeouts. |
| `desktop::Desktop` | Host opt-in, exclusive connection lease, bounded ordered UI inputs and latest shared image/explicit clipboard output. |

The optional desktop extension adds `pair_desktop`, `desktop_open`,
`desktop_input`, `desktop_frame` and `desktop_clipboard`. Original protocol
v1 messages retain their exact wire representation. A host without the
extension refuses the new message instead of executing a partial command.
Native tokens are held only in the controller process and use the same
authentication and rotation flow as phone tokens.

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
