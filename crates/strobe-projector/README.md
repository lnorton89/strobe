# strobe-projector

Projector power over the network for the show PC (D502): PJLink Class 1 on
TCP 4352, with Panasonic's NTCONTROL on TCP 1024 as a fallback, a
token-protected web page a phone opens, and the `strobe-projector` binary
(`pj` once installed) as the CLI and the headless service. It is the
Rust rewrite of the Python tool that PR #45 proposed, with the same
behaviour command for command. Strobe uses it in-process for
Settings > Projectors.

## The power sequence

[`unit`](src/unit.rs) is the part that matters on the night:

- no power command while a unit is warming (3) or cooling (2): it waits,
  then reads the state again right before sending;
- nothing sent when the unit is already in the requested state;
- ERR3 and refused connections retried with a 1 s backoff doubling to
  10 s, for up to 90 s;
- `--wait` polls every 3 s for up to 300 s (the PT-DZ6700 cools for about
  172 s), and for "on" over PJLink also waits for a lamp-lit flag in
  `LAMP ?`, because the PT-DZ6700 reports on about 21 s before its lamps
  strike;
- a unit configured with `"fallback": "ntcontrol"` tries NTCONTROL when
  PJLink is unusable on it (a wrong password, an unsupported command, a
  refused or silent port), never on ERR3.

## Modules

| Module | |
|---|---|
| [`wire`](src/wire.rs) | TCP with connect and read timeouts, CR-terminated lines, greetings, the MD5 digest |
| [`pjlink`](src/pjlink.rs), [`ntcontrol`](src/ntcontrol.rs) | The two clients, one connection per command |
| [`unit`](src/unit.rs) | The fallback, the retry policy, the safe sequence, waiting, status |
| [`config`](src/config.rs) | `projectors.json` in Strobe's data directory, never in the repository |
| [`audit`](src/audit.rs) | `projector-logs/commands.log` beside the config, rotated at 1 MB, ten kept |
| [`discover`](src/discover.rs), [`dhcp`](src/dhcp.rs) | The subnet and ARP scan, the passive DHCP listen, and the reservation responder |
| [`server`](src/server/mod.rs) | The page and JSON API on its own small HTTP/1.1 subset, like `strobe-remote`'s |
| [`bind`](src/bind.rs), [`service`](src/service.rs) | The Tailscale or Wi-Fi address, never `0.0.0.0`, and the supervisor that follows it |

## The CLI

```text
pj status   [all|key]          power, input, lamp/laser hours, error status
pj on       [all|key] [--wait]
pj off      [all|key] [--wait]
pj discover [--subnet CIDR]    scan + ARP; on problems, listen 15 s for DHCP clients
pj raw      key "%1NAME ?"
pj serve                       the web page (the Scheduled Task's command)
pj dhcp-reserve [--seconds N]  answer DHCP for projectors with a `mac`
pj --config PATH -v ...
```

Exit codes: `0` ok, `1` a projector or command failed, `2` a usage or
configuration error. Installing it as a service is
[`tools/projector-control`](../../tools/projector-control/README.md).

## Dependencies

`md-5` for the authentication digest (the same RustCrypto `digest` 0.11 as
`sha1` and `sha2` in the tree, so it adds one crate), `serde_json` for the
config and the API, `getrandom` for the token, `chrono` for the log's local
time, `clap` for the CLI, and `strobe-diag` for the data directory and the
service's log. No HTTP framework, no async runtime.

## Status and known gaps

Everything above is tested against fake PJLink and NTCONTROL projectors on
loopback (`tests/fake_projector.rs`), and the web API over real sockets.

- The Rust build has not yet run against the venue's PT-DZ6700 and
  VPL-PHZ10; the Python tool it replaces had (backlog L-09).
- The NTCONTROL fallback has never switched a real unit, in either
  language (backlog L-09).
- One command at a time per unit holds within a process; Strobe and the
  headless service are two processes and can each command a unit at once
  (backlog L-09).
- The page is plain HTTP, for the venue Wi-Fi or the tailnet only; by
  design (D502), as the LAN remote's is until L-07.
