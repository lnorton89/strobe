# Projector Control

Network power control for the venue's two projectors from the Strobe PC.
PJLink Class 1 on TCP 4352, with a Panasonic NTCONTROL fallback on TCP 1024.
Python 3 standard library only. Windows (Scheduled Task service).

The tool is separate from the Strobe engine and shares no code with it. It lives in this repository so the
show machine's setup is versioned in one place.

## Install

From an **elevated** PowerShell in this directory:

```powershell
.\setup-network.ps1     # Ethernet -> 192.168.50.1/24, no gateway/DNS, metric above Wi-Fi (idempotent)
.\install.ps1           # deploy to C:\ProgramData\ProjectorControl and start the service (idempotent)
pj discover             # both projectors found at their IPs, no PROBLEM lines
pj status
```

The first `install.ps1` run creates `C:\ProgramData\ProjectorControl\config\config.json` from
`config.example.json`, with a freshly generated API token. Fill in the projector passwords, then
re-run `install.ps1`, or `Restart-ScheduledTask 'ProjectorControl API'`.
Re-run `install.ps1` after pulling changes to this directory.

`install.ps1` vendors the newest CPython that the `py` launcher finds into
`C:\ProgramData\ProjectorControl\python`, because the SYSTEM service can't run Store aliases or
per-user installs. Pass `-PythonHome <dir>` to choose a different one.

Run tests with `python tests\test_pjctl.py`. It runs offline, against a fake PJLink projector.

## Interface and IP layout

| Interface | Role | Address | Gateway | DNS | Metric |
|---|---|---|---|---|---|
| `Wi-Fi` | Internet, remote access, **default route** | DHCP | DHCP | DHCP | auto (lower) |
| `Ethernet` | Projector LAN through a dumb switch | **192.168.50.1/24** static | none | none | 500 (fixed) |

| Projector | IP | Protocol |
|---|---|---|
| Panasonic PT-DZ6700 | 192.168.50.10 | PJLink 4352, falls back to NTCONTROL 1024 |
| Sony VPL-PHZ10 | 192.168.50.11 | PJLink 4352 |

`setup-network.ps1` never touches Wi-Fi.
It forces DHCP off in the registry, because Windows ignores `netsh` and `Set-NetIPInterface` while the cable is unplugged.
It also clears stale DHCP-lease DNS values left from older networks.

Windows drops an on-link route whose neighbour doesn't answer ARP and falls back to the default route.
So `Find-NetRoute 192.168.50.10` showing Wi-Fi means *the projector isn't answering*. It doesn't mean the routing is wrong.

## Files (installed)

| Path | What |
|---|---|
| `config\config.json` | Names, IPs, protocol, passwords, API token. **Only SYSTEM (the service user) and Administrators can read it.** |
| `app\pjctl.py` | PJLink/NTCONTROL client, discovery, CLI |
| `app\server.py`, `app\ui.html` | HTTP API and single-page dark UI |
| `app\dhcp_reserve.py` | One-shot DHCP responder on 192.168.50.1:67. It answers **only** projectors with a `mac` in the config, and offers no router or DNS. It rescues a unit left in DHCP mode: `python\python.exe app\dhcp_reserve.py --seconds 60`. A static IP on the unit is the permanent fix. |
| `logs\commands.log` | Audit log: timestamp, source IP, target, action, result. Rotates at 1 MB, keeps 10. |
| `logs\server.log` | Service log. Rotates at 1 MB, keeps 5. |

## CLI

The config is admin-only, so run these from an elevated shell. `pj` is on the machine PATH.

```
pj status [all|panasonic|sony]          power, input, lamp/laser hours, error status
pj on     [all|panasonic|sony] [--wait]  --wait polls until on (and a lamp reports lit)
pj off    [all|panasonic|sony] [--wait]  --wait polls through cooling until off
pj discover                               scan 192.168.50.0/24 + ARP; on problems, listen for DHCP clients
pj raw sony "%1NAME ?"                    send a raw PJLink command
pj -v ...                                 verbose (retries, fallbacks)
```

Exit codes: `0` ok, `1` projector or command failure, `2` config error.

A power command is never sent while a unit is warming (3) or cooling (2). The tool waits for a
stable state, rechecks right before sending, and retries ERR3 and connection refusals with
backoff for up to 90 s. If the unit is already in the requested state, nothing is sent.

## HTTP API and UI

The service is the Scheduled Task **"ProjectorControl API"**. It runs as SYSTEM at startup (30 s delay)
whether or not anyone is logged in, and restarts every minute on failure.

It binds **only** to the Tailscale IPv4 if Tailscale is up, otherwise to the Wi-Fi IPv4.
It never binds to 0.0.0.0, loopback, link-local or the projector subnet, and it rebinds within 30 s if that address changes.
The firewall allows the port on the Private profile, for the vendored python.exe only.
**Never port-forward it.**

- **UI:** `http://<wifi-or-tailscale-ip>:8780/`. Paste the token once; it's kept in an HttpOnly, SameSite=Strict cookie.
- **Token:** `api.token` in `config.json`. Every endpoint requires it.

| Method | Path | |
|---|---|---|
| GET | `/` | UI (401 login form without a token) |
| GET | `/api/units` | configured projectors |
| GET | `/api/status`, `/api/status/{all\|panasonic\|sony}` | status JSON |
| POST | `/api/power/{all\|panasonic\|sony}/{on\|off}` | power. 200 ok, 502 if any unit failed |

Auth: `Authorization: Bearer <token>`, or the `pj_token` cookie set by the UI.

## Projector-side checklist

- [ ] **Panasonic:** MENU → NETWORK → LAN SETTINGS: DHCP OFF, IP 192.168.50.10, mask 255.255.255.0, gateway blank → STORE. Its factory IP is 192.168.0.8; to reach the web UI there, add a temporary 192.168.0.x address to Ethernet.
- [ ] **Panasonic:** PROJECTOR SETUP → STANDBY MODE = **NORMAL**. With ECO, *"NETWORK function, RS-232C output and some of RS-232C commands are not available"* (Functional Instructions p.29). The setting is only in the on-screen menu, but the web UI's *Detail control* virtual remote can open it.
- [ ] **Panasonic:** network control enabled. Its PJLink password is the web admin password.
- [ ] **Sony:** Network settings: DHCP off, IP 192.168.50.11, mask 255.255.255.0 (in the web UI or Connection/Power menu).
- [ ] **Sony:** Standby Mode = **Standard**. Low turns network control off in standby.
- [ ] **Sony:** PJ Link Service enabled. The VPL-PHZ10 kept asking for authentication even with auth shown as disabled in its web UI, and accepted the PJLink spec default password.
- [ ] **Both:** the web UI loads at the projector's IP from the PC.

## Verified behaviour (2026-10-27, on the real units)

| | Panasonic PT-DZ6700 | Sony VPL-PHZ10 |
|---|---|---|
| Off → state 0 | ~172 s (lamp cool-down) | ~3–4 s |
| On → lit | POWR reports 1 straight away; lamps lit after ~21 s. `--wait` waits for the lamp flag. | ~16 s |
| PJLink in standby / LAN wake | yes / yes | yes / yes |

The service also survived a reboot: it came back with nobody logged in and served status.

## Troubleshooting

**ERRA (authentication failed).** The PJLink password in `config.json` is wrong.
An empty string means no password, but some units still require one; see the Sony note above.
On the Panasonic, an ERRA also triggers the NTCONTROL fallback.

**ERR3 (unavailable time).** The unit is warming or cooling. The tool already waits and retries for up to 90 s.
Lamp units can take minutes to cool, so re-run once it's finished.

**No response in standby, but fine when on.** The standby mode setting is wrong: set STANDBY MODE to NORMAL on the Panasonic, or Standard on the Sony.

**Projector not found by `pj discover`.**
- "DHCP client … vendor=…" means the unit is in DHCP mode on a LAN with no DHCP server. Set the static IP, or use `dhcp_reserve.py` for now.
- If nothing appears at all, the unit is on a static IP outside the subnet, or check the cabling and mains power.

**Internet stops working.** `Get-NetRoute -DestinationPrefix 0.0.0.0/0` must show only Wi-Fi. Re-run `setup-network.ps1`.
