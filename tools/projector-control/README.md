# Projector Control

Network power control for the venue's two projectors from the Strobe PC.
PJLink Class 1 on TCP 4352, with a Panasonic NTCONTROL fallback on TCP 1024.
Windows (Scheduled Task service).

The code is the [`strobe-projector`](../../crates/strobe-projector/README.md)
crate (D502). Strobe uses it directly for **Settings > Projectors**. This
directory holds what sets up the show PC around it:

- `setup-network.ps1`: the projector Ethernet;
- `install.ps1`: `pj` and the headless service;
- `config.example.json`: the config the installer starts from.

It replaces the Python tool of PR #45, whose behaviour it keeps command for
command.

## Two ways to run it

- **From Strobe:** Settings > Projectors. Switch on *Projector control*, add
  the projectors under *Connections* and Save, then switch them with On and
  Off. Switch on *Web page* to serve the phone page while Strobe runs.
- **Headless:** `install.ps1` registers `strobe-projector serve` as a
  startup task that runs as the user who runs Strobe (D518). The page is
  then there whenever the PC is on, with Strobe closed or not and nobody
  logged in.

Both read the same file, `projectors.json` in Strobe's data directory
(`%LOCALAPPDATA%\strobe\projectors.json`). Whichever binds the address
first serves the page; the other says the port is in use. With the task
installed, leave Strobe's *Web page* switch off.

## Install

From an **elevated** PowerShell in this directory, signed in as the user
who runs Strobe:

```powershell
.\setup-network.ps1     # Ethernet -> 192.168.50.1/24, no gateway/DNS, metric above Wi-Fi (idempotent)
.\install.ps1           # build, install pj, start the service (idempotent)
pj discover             # both projectors found at their IPs, no PROBLEM lines
pj status
```

If PowerShell refuses to run the scripts because of the execution policy
(the mini PC's does), run each one as
`powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1`. That
bypasses the policy for that one run and changes no setting.

`install.ps1` does the following:

1. It builds `strobe-projector.exe` with `cargo build --release`, or takes
   `-Exe <path>`.
2. It copies the binary and its supervisor, `pj-service.ps1`, to
   `projector-service\` beside the config (in that user's profile, so they
   own it), and puts `pj` on the machine PATH, pointed at the config.
3. It creates the config if it is missing:
   - **an existing Python install's `config\config.json` is moved over as
     it is**, keeping the token, the passwords and the Sony's MAC;
   - otherwise the config comes from `config.example.json` with a fresh
     token.
4. It opens the page's port on the Private profile for the binary only, and
   registers the task: at startup, as the user running the installer (or
   `-ServiceUser`), with S4U so no password is stored, and not elevated.

`-Config <path>` uses another config. `-NoService` installs `pj` only and
removes the task; Strobe then serves the page.
`-StrobeExe <path>` also lets Strobe's own `strobe.exe` through the firewall
on the page's port.

**Upgrading from the Python install:** run `install.ps1` once. It stops the
old Python server and replaces its task (same name, "ProjectorControl
API"). It removes the old `app\` and `python\`, and moves the config. The
old `config\` (admin-only) and `logs\` folders stay in
`C:\ProgramData\ProjectorControl` until you delete them.

**Upgrading from the SYSTEM task (D502):** run `install.ps1` once more, as
the user who runs Strobe. It stops the SYSTEM task's binary, removes
`C:\ProgramData\ProjectorControl\bin`, and registers the task as that user.

## Updates

The task runs `pj-service.ps1`, which keeps `strobe-projector serve` running
and, before each start, moves `projector-service\next\strobe-projector.exe`
over the running copy. An update is: build, copy the exe into `next\`, end
the running `strobe-projector`. None of it needs elevation, and the binary
keeps its path, so the firewall rules hold. The supervisor logs each swap
and restart to `projector-logs\service-updates.log`.

On the mini PC the show launcher does this at every logon, right after it
builds Strobe. These lines go in `jaja\start-show.ps1` (local to that PC),
between step 3, the build, and step 4, the scene check:

```powershell
# 3b. The projector page's service (D518): build it from the same checkout and hand a new build
#     to its supervisor, which swaps it in and starts it again. A failed build keeps the running one.
$PjDir   = Join-Path $env:LOCALAPPDATA 'strobe\projector-service'
$PjExe   = Join-Path $PjDir 'strobe-projector.exe'
$PjBuilt = Join-Path $Repo 'target\release-fast\strobe-projector.exe'
$pjBuild = & $Cargo build --profile release-fast -p strobe-projector --manifest-path (Join-Path $Repo 'Cargo.toml') 2>&1 | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) {
    Log "projector service: BUILD FAILED, keeping the running one; first error: $(($pjBuild | Select-String '^error' | Select-Object -First 1).Line)"
} elseif (-not (Test-Path $PjExe)) {
    Log "projector service: nothing installed at $PjExe (run tools\projector-control\install.ps1 once, elevated); not updating"
} elseif ((Get-FileHash $PjBuilt).Hash -eq (Get-FileHash $PjExe).Hash) {
    Log 'projector service: already running this build'
} else {
    New-Item -ItemType Directory -Force (Join-Path $PjDir 'next') | Out-Null
    Copy-Item $PjBuilt (Join-Path $PjDir 'next\strobe-projector.exe') -Force
    Get-CimInstance Win32_Process -Filter "Name='strobe-projector.exe'" |
        Where-Object { $_.ExecutablePath -eq $PjExe -and $_.CommandLine -like '* serve*' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    Log 'projector service: new build handed to its supervisor'
}
```

The first logon after an install swaps once even with no code change,
because `install.ps1` builds `release` and the launcher `release-fast`.
Anywhere else, re-run `install.ps1` after pulling changes, or do the three
steps by hand.

The tests run offline against fake projectors:

```powershell
cargo test -p strobe-projector
```

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

## Files

| Path | What |
|---|---|
| `%LOCALAPPDATA%\strobe\projectors.json` | Names, IPs, passwords, the page's bind, port and token, and Strobe's two switches. In the user's profile: that user, SYSTEM and Administrators can read it. |
| `%LOCALAPPDATA%\strobe\projector-logs\commands.log` | Audit log: timestamp, source (`cli:<user>`, `strobe`, `dhcp` or the client's IP), target, action, result. Rotates at 1 MB, keeps 10. The service's own log is beside it. |
| `%LOCALAPPDATA%\strobe\projector-service\` | The binary (CLI, service and DHCP helper), `pj-service.ps1`, and `next\` for an update. That user has full control. |
| `%LOCALAPPDATA%\strobe\projector-logs\service-updates.log` | The supervisor's log: each swap and restart. |
| `C:\ProgramData\ProjectorControl\pj.cmd` | `pj`, on the machine PATH |

## CLI

`pj` is on the machine PATH.

```
pj status   [all|panasonic|sony]          power, input, lamp/laser hours, error status
pj on       [all|panasonic|sony] [--wait] --wait polls until on (and a lamp reports lit)
pj off      [all|panasonic|sony] [--wait] --wait polls through cooling until off
pj discover                               scan 192.168.50.0/24 + ARP; on problems, listen for DHCP clients
pj raw sony "%1NAME ?"                    send a raw PJLink command
pj dhcp-reserve --seconds 60              answer DHCP for projectors with a "mac" (rescue only)
pj -v ...                                 verbose (retries, fallbacks)
```

Exit codes: `0` ok, `1` projector or command failure, `2` config error.

A power command is never sent while a unit is warming (3) or cooling (2). The tool waits for a
stable state, rechecks right before sending, and retries ERR3 and connection refusals with
backoff for up to 90 s. If the unit is already in the requested state, nothing is sent.

`pj dhcp-reserve` is a one-shot DHCP responder on 192.168.50.1:67. It answers **only** projectors
with a `mac` in the config, and offers no router or DNS. It rescues a unit left in DHCP mode;
a static IP on the unit is the permanent fix.

## Web page and API

The service is the Scheduled Task **"ProjectorControl API"**. It runs `pj-service.ps1`, which
keeps `strobe-projector serve` running (again 3 s after it exits, or 30 s if it lasted under
10 s), as the user who runs Strobe (S4U, not elevated) at startup (30 s delay), whether or not
anyone is logged in. The task restarts the supervisor every minute
on failure.

The page binds **only** to the Tailscale IPv4 if Tailscale is up, otherwise to the Wi-Fi IPv4.
It never binds to 0.0.0.0, loopback, link-local or the projector subnet, and it rebinds within 30 s if that address changes.
The firewall allows the port on the Private profile, for the binary only.
**Never port-forward it.**

- **UI:** `http://<wifi-or-tailscale-ip>:8780/`. Strobe's Settings > Projectors has *Copy link*. A phone signs in with the password once and keeps a session cookie (HttpOnly, SameSite=Strict) made from it, not the password itself.
- **Password:** `api.password` in `projectors.json`, at least 6 characters (D516). Edit it there or as *Page password* in Settings; a running service picks it up within 10 s, and changing it signs every phone out. Every endpoint but `/favicon.ico` and the web app's manifest, worker and icons (D517) requires it. A file from before D516 has `api.token`, which is read as the password until one is set.

| Method | Path | |
|---|---|---|
| GET | `/login` | `Authorization: Bearer <password>` for the session cookie: 204, or 401 |
| GET | `/` | UI (401 password form without the cookie) |
| GET | `/api/units` | configured projectors |
| GET | `/api/status`, `/api/status/{all\|panasonic\|sony}` | status JSON |
| POST | `/api/power/{all\|panasonic\|sony}/{on\|off}` | power. 200 ok, 502 if any unit failed |

Auth: `Authorization: Bearer <password>`, or the `pj_session` cookie `/login` sets.

## Projector-side checklist

- [ ] **Panasonic:** MENU → NETWORK → LAN SETTINGS: DHCP OFF, IP 192.168.50.10, mask 255.255.255.0, gateway blank → STORE. Its factory IP is 192.168.0.8; to reach the web UI there, add a temporary 192.168.0.x address to Ethernet.
- [ ] **Panasonic:** PROJECTOR SETUP → STANDBY MODE = **NORMAL**. With ECO, *"NETWORK function, RS-232C output and some of RS-232C commands are not available"* (Functional Instructions p.29). The setting is only in the on-screen menu, but the web UI's *Detail control* virtual remote can open it.
- [ ] **Panasonic:** network control enabled. Its PJLink password is the web admin password.
- [ ] **Sony:** Network settings: DHCP off, IP 192.168.50.11, mask 255.255.255.0 (in the web UI or Connection/Power menu).
- [ ] **Sony:** Standby Mode = **Standard**. Low turns network control off in standby.
- [ ] **Sony:** PJ Link Service enabled. The VPL-PHZ10 kept asking for authentication even with auth shown as disabled in its web UI, and accepted the PJLink spec default password.
- [ ] **Both:** the web UI loads at the projector's IP from the PC.

## Verified behaviour (2026-10-27, the Python tool on the real units)

| | Panasonic PT-DZ6700 | Sony VPL-PHZ10 |
|---|---|---|
| Off → state 0 | ~172 s (lamp cool-down) | ~3–4 s |
| On → lit | POWR reports 1 straight away; lamps lit after ~21 s. `--wait` waits for the lamp flag. | ~16 s |
| PJLink in standby / LAN wake | yes / yes | yes / yes |

The service also survived a reboot: it came back with nobody logged in and served status.

### `strobe-projector` on the real units (2026-10-27, master e61f35d, Task M30)

Installed on the mini PC over the Python install with `install.ps1`. The config moved
over byte-identical.

Power, with `pj -v on|off all --wait`, run twice. The second "on" came from standby, which is the LAN wake check:

| | Panasonic PT-DZ6700 | Sony VPL-PHZ10 |
|---|---|---|
| On → reached | 20.1 s / 20.0 s. States seen: `on`, `lamp starting`, `on`. | 15.6 s / 15.6 s. States seen: `off`, `warming`, `on`. |
| Off → state 0 | 171.3 s / 171.3 s. States seen: `cooling`, `off`. | 3.5 s / 3.5 s |
| `%1POWR ?` in standby (3 probes, 15 s apart) | `0` every time | `0` every time |
| Wake from standby | yes | yes |

The other checks:

- `pj discover` found both at their IPs (`PANASONIC-PT` / `DZ6700`, `VPL-PHZ10` / `SONY`),
  with no PROBLEM lines.
- `pj status` read power, input, lamp hours and errors from both over PJLink. Both were off at the time.
- The task runs `strobe-projector.exe --config …\projectors.json serve` as SYSTEM. It listens
  only on the Wi-Fi address, port 8780.
- The page and API return 401 with no token, a wrong token, or an unauthenticated POST,
  and 200 with the token.

**Not yet run with the Rust build (backlog L-09):** the reboot check, that the task
comes back with nobody logged in.

## Troubleshooting

**ERRA (authentication failed).** The PJLink password in the config is wrong.
An empty string means no password, but some units still require one; see the Sony note above.
On the Panasonic, an ERRA also triggers the NTCONTROL fallback.

**ERR3 (unavailable time).** The unit is warming or cooling. The tool already waits and retries for up to 90 s.
Lamp units can take minutes to cool, so re-run once it's finished.

**No response in standby, but fine when on.** The standby mode setting is wrong: set STANDBY MODE to NORMAL on the Panasonic, or Standard on the Sony.

**Strobe's Web page switch says the port is in use.** The task is serving the page; that is expected. Use the task's page, or run `install.ps1 -NoService` to let Strobe serve it.

**Projector not found by `pj discover`.**
- "DHCP client … vendor=…" means the unit is in DHCP mode on a LAN with no DHCP server. Set the static IP, or use `pj dhcp-reserve` for now.
- If nothing appears at all, the unit is on a static IP outside the subnet, or check the cabling and mains power.

**Internet stops working.** `Get-NetRoute -DestinationPrefix 0.0.0.0/0` must show only Wi-Fi. Re-run `setup-network.ps1`.
