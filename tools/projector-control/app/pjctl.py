"""Projector power control: PJLink Class 1 client, Panasonic NTCONTROL fallback,
discovery, and the `pj` CLI. Python 3 standard library only.

Exit codes: 0 ok, 1 command/projector failure, 2 usage/config error.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import getpass
import hashlib
import ipaddress
import json
import logging
import logging.handlers
import os
import re
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

INSTALL_DIR = Path(os.environ.get("PJ_HOME", r"C:\ProgramData\ProjectorControl"))
CONFIG_PATH = Path(os.environ.get("PJ_CONFIG", INSTALL_DIR / "config" / "config.json"))
LOG_DIR = INSTALL_DIR / "logs"

PJLINK_PORT = 4352
NTCONTROL_PORT = 1024
CONNECT_TIMEOUT = 4.0      # s, TCP connect
IO_TIMEOUT = 6.0           # s, per read
RETRY_BUDGET = 90.0        # s, total for ERR3 / connection refused
POLL_INTERVAL = 3.0        # s, state polling

POWER_STATES = {"0": "off", "1": "on", "2": "cooling", "3": "warming"}
TRANSITIONAL = {"2", "3"}

PJLINK_ERRORS = {
    "ERRA": "authentication failed (wrong PJLink password)",
    "ERR1": "undefined command (projector does not support it)",
    "ERR2": "out of parameter (invalid argument)",
    "ERR3": "unavailable time (projector is warming up or cooling down)",
    "ERR4": "projector failure (hardware fault, check the unit)",
}

INPUT_TYPES = {"1": "RGB", "2": "VIDEO", "3": "DIGITAL", "4": "STORAGE", "5": "NETWORK"}
ERST_FIELDS = ["fan", "lamp/light", "temperature", "cover", "filter", "other"]
ERST_LEVELS = {"0": "ok", "1": "WARNING", "2": "ERROR"}


# --------------------------------------------------------------------------- errors

class PJError(Exception):
    """Any failure talking to a projector."""


class PJLinkError(PJError):
    def __init__(self, code: str, command: str):
        self.code = code
        super().__init__(f"{command}: {code} = {PJLINK_ERRORS.get(code, 'unknown error')}")


class TransportError(PJError):
    """Could not connect / no response."""


class ConfigError(Exception):
    pass


# --------------------------------------------------------------------------- logging

_cmd_logger: logging.Logger | None = None
_log_lock = threading.Lock()


def command_log() -> logging.Logger:
    """Rotating audit log: timestamp, source, target, action, result."""
    global _cmd_logger
    with _log_lock:
        if _cmd_logger is None:
            lg = logging.getLogger("pj.commands")
            lg.setLevel(logging.INFO)
            lg.propagate = False
            try:
                LOG_DIR.mkdir(parents=True, exist_ok=True)
                h: logging.Handler = logging.handlers.RotatingFileHandler(
                    LOG_DIR / "commands.log", maxBytes=1_000_000, backupCount=10, encoding="utf-8")
            except OSError:
                h = logging.StreamHandler(sys.stderr)  # not admin: still log somewhere
            h.setFormatter(logging.Formatter("%(asctime)s\t%(message)s", "%Y-%m-%dT%H:%M:%S%z"))
            lg.addHandler(h)
            _cmd_logger = lg
    return _cmd_logger


def audit(source: str, target: str, action: str, result: str) -> None:
    command_log().info("src=%s\ttarget=%s\taction=%s\tresult=%s", source, target, action, result)


# --------------------------------------------------------------------------- config

def load_config(path: Path = CONFIG_PATH) -> dict:
    try:
        with open(path, encoding="utf-8") as f:
            cfg = json.load(f)
    except FileNotFoundError as e:
        raise ConfigError(f"config not found: {path}") from e
    except PermissionError as e:
        raise ConfigError(f"cannot read {path}: run from an elevated (Administrator) shell") from e
    except json.JSONDecodeError as e:
        raise ConfigError(f"invalid JSON in {path}: {e}") from e
    if not isinstance(cfg.get("projectors"), dict) or not cfg["projectors"]:
        raise ConfigError(f"{path}: 'projectors' section missing or empty")
    for key, p in cfg["projectors"].items():
        for field in ("name", "ip", "protocol"):
            if field not in p:
                raise ConfigError(f"{path}: projector '{key}' missing '{field}'")
    return cfg


# --------------------------------------------------------------------------- socket helpers

def _read_line(sock: socket.socket, timeout: float = IO_TIMEOUT) -> str:
    sock.settimeout(timeout)
    buf = bytearray()
    while True:
        try:
            chunk = sock.recv(1)
        except socket.timeout as e:
            raise TransportError(f"timeout waiting for response (got {bytes(buf)!r})") from e
        if not chunk:
            if buf:
                break
            raise TransportError("connection closed by projector")
        if chunk == b"\r":
            break
        buf += chunk
        if len(buf) > 512:
            raise TransportError("response too long")
    return buf.decode("ascii", errors="replace").strip("\n")


def _connect(ip: str, port: int) -> socket.socket:
    try:
        return socket.create_connection((ip, port), timeout=CONNECT_TIMEOUT)
    except ConnectionRefusedError:
        raise
    except socket.timeout as e:
        raise TransportError(f"{ip}:{port} connect timeout (no response)") from e
    except OSError as e:
        raise TransportError(f"{ip}:{port} {e.strerror or e}") from e


# --------------------------------------------------------------------------- PJLink

class PJLink:
    """PJLink Class 1. One TCP session per command (simplest, works on every unit)."""

    def __init__(self, ip: str, password: str = "", port: int = PJLINK_PORT):
        self.ip, self.password, self.port = ip, password or "", port

    def raw(self, body: str) -> str:
        """Send '%1XXXX param' and return the value after '='. Raises PJLinkError on ERRx."""
        cmd = body if body.startswith("%1") else "%1" + body
        sock = _connect(self.ip, self.port)
        try:
            greeting = _read_line(sock)
            if greeting == "PJLINK ERRA":
                raise PJLinkError("ERRA", cmd)
            m = re.fullmatch(r"PJLINK ([01])(?: (\S+))?", greeting)
            if not m:
                raise TransportError(f"unexpected greeting {greeting!r} (not a PJLink device?)")
            prefix = ""
            if m.group(1) == "1":
                if not m.group(2):
                    raise TransportError("auth greeting without random")
                prefix = hashlib.md5((m.group(2) + self.password).encode("ascii")).hexdigest()
            sock.sendall((prefix + cmd + "\r").encode("ascii"))
            resp = _read_line(sock)
        finally:
            sock.close()
        if resp == "PJLINK ERRA":
            raise PJLinkError("ERRA", cmd)
        head = cmd[:6]  # %1POWR
        if not resp.upper().startswith(head.upper() + "="):
            raise TransportError(f"unexpected response {resp!r} to {cmd!r}")
        val = resp[len(head) + 1:]
        if val in PJLINK_ERRORS:
            raise PJLinkError(val, cmd)
        return val

    # queries
    def power(self) -> str:
        return self.raw("%1POWR ?")

    def set_power(self, on: bool) -> None:
        val = self.raw("%1POWR " + ("1" if on else "0"))
        if val != "OK":
            raise PJError(f"POWR set returned {val!r}")

    def name(self) -> str:
        return self.raw("%1NAME ?")

    def inf1(self) -> str:
        return self.raw("%1INF1 ?")

    def inf2(self) -> str:
        return self.raw("%1INF2 ?")

    def erst(self) -> str:
        return self.raw("%1ERST ?")

    def lamp(self) -> str:
        return self.raw("%1LAMP ?")

    def input(self) -> str:
        return self.raw("%1INPT ?")


# --------------------------------------------------------------------------- NTCONTROL (Panasonic)

class NTControl:
    """Panasonic native protocol, TCP 1024. Fallback only.

    Greeting: 'NTCONTROL 0' (no auth) or 'NTCONTROL 1 <nonce>'.
    Command:  [md5hex('user:pass:nonce')] + '00' + CMD + CR.  Reply: '00' + data, or ERRx.
    """

    ERRORS = {"ERRA": "authentication failed (web UI user/password)",
              "ER401": "command cannot be executed now (warming/cooling or wrong state)",
              "ER402": "invalid parameter",
              "ERR1": "undefined command", "ERR2": "out of parameter",
              "ERR3": "busy", "ERR4": "timeout", "ERR5": "wrong data length"}

    def __init__(self, ip: str, user: str, password: str, port: int = NTCONTROL_PORT):
        self.ip, self.user, self.password, self.port = ip, user, password, port

    def raw(self, cmd: str) -> str:
        sock = _connect(self.ip, self.port)
        try:
            greeting = _read_line(sock)
            m = re.fullmatch(r"NTCONTROL ([01])(?: (\S+))?", greeting)
            if not m:
                raise TransportError(f"unexpected NTCONTROL greeting {greeting!r}")
            prefix = ""
            if m.group(1) == "1":
                prefix = hashlib.md5(f"{self.user}:{self.password}:{m.group(2)}".encode()).hexdigest()
            sock.sendall((prefix + "00" + cmd + "\r").encode("ascii"))
            resp = _read_line(sock)
        finally:
            sock.close()
        for code, msg in self.ERRORS.items():
            if code in resp:
                raise PJError(f"NTCONTROL {cmd}: {code} = {msg}")
        return resp[2:] if resp.startswith("00") else resp

    def power(self) -> str:
        """Map QPW (000 standby / 001 on) to PJLink codes '0'/'1'."""
        val = self.raw("QPW")
        digits = re.sub(r"\D", "", val)
        if not digits:
            raise PJError(f"NTCONTROL QPW: unexpected reply {val!r}")
        return "1" if digits.endswith("1") else "0"

    def set_power(self, on: bool) -> None:
        self.raw("PON" if on else "POF")


# --------------------------------------------------------------------------- unit + retry policy

class Unit:
    _locks: dict[str, threading.Lock] = {}

    def __init__(self, key: str, conf: dict):
        self.key, self.conf = key, conf
        self.name = conf["name"]
        self.ip = conf["ip"]
        self.pjlink = PJLink(self.ip, conf.get("pjlink_password", ""))
        self.nt = None
        if conf.get("fallback") == "ntcontrol":
            self.nt = NTControl(self.ip, conf.get("ntcontrol_user", "admin1"),
                                conf.get("ntcontrol_password", ""))
        self.lock = Unit._locks.setdefault(key, threading.Lock())
        self.via = "pjlink"

    # Any failure that means "PJLink is not working on this unit" (not a state answer).
    @staticmethod
    def _pjlink_broken(e: Exception) -> bool:
        if isinstance(e, PJLinkError):
            return e.code in ("ERRA", "ERR1")
        return isinstance(e, (TransportError, ConnectionRefusedError))

    def _call(self, fn_name: str, *args):
        """Run a power call over PJLink, falling back to NTCONTROL if PJLink is unusable."""
        try:
            self.via = "pjlink"
            return getattr(self.pjlink, fn_name)(*args)
        except (PJError, ConnectionRefusedError) as e:
            if self.nt is None or not self._pjlink_broken(e):
                raise
            log.warning("%s: PJLink failed (%s); falling back to NTCONTROL", self.key, e)
            self.via = "ntcontrol"
            return getattr(self.nt, fn_name)(*args)

    def _with_retry(self, fn, what: str, deadline: float):
        """Retry on ERR3 and connection refused with backoff until the deadline."""
        delay = 1.0
        while True:
            try:
                return fn()
            except PJLinkError as e:
                if e.code != "ERR3" or time.monotonic() + delay > deadline:
                    raise
                log.info("%s: %s -> ERR3, retrying in %.0fs", self.key, what, delay)
            except ConnectionRefusedError as e:
                if time.monotonic() + delay > deadline:
                    raise TransportError(f"{self.ip}: connection refused for {RETRY_BUDGET:.0f}s") from e
                log.info("%s: %s -> connection refused, retrying in %.0fs", self.key, what, delay)
            time.sleep(delay)
            delay = min(delay * 2, 10.0)

    def power_state(self, deadline: float | None = None) -> str:
        deadline = deadline or time.monotonic() + RETRY_BUDGET
        return self._with_retry(lambda: self._call("power"), "POWR ?", deadline)

    def set_power(self, on: bool, wait: bool = False, wait_timeout: float = 300.0) -> dict:
        """Power on/off safely. Never sends POWR while warming/cooling."""
        target = "1" if on else "0"
        t0 = time.monotonic()
        with self.lock:
            deadline = t0 + RETRY_BUDGET
            # 1. wait out any transition
            state = self.power_state(deadline)
            while state in TRANSITIONAL:
                if time.monotonic() > deadline:
                    raise PJError(f"still {POWER_STATES[state]} after {RETRY_BUDGET:.0f}s; not sending power command")
                log.info("%s: %s, waiting before sending power command", self.key, POWER_STATES[state])
                time.sleep(POLL_INTERVAL)
                state = self.power_state(deadline)
            sent = False
            if state != target:
                def send():
                    # re-check right before sending: never command during a transition
                    s = self._call("power")
                    if s in TRANSITIONAL:
                        raise PJLinkError("ERR3", "%1POWR (pre-check)")
                    if s != target:
                        self._call("set_power", on)
                    return s
                self._with_retry(send, "POWR set", deadline)
                sent = True
            result = {"unit": self.key, "sent": sent, "previous": POWER_STATES.get(state, state), "via": self.via}
            if wait:
                result.update(self._wait_for(target, t0 + wait_timeout))
            result["elapsed_s"] = round(time.monotonic() - t0, 1)
            return result

    def _wait_for(self, target: str, deadline: float) -> dict:
        seen = []
        while True:
            s = self.power_state(max(deadline, time.monotonic() + 15))
            name = POWER_STATES.get(s, s)
            if not seen or seen[-1] != name:
                seen.append(name)
            if s == target:
                # Some units (the PT-DZ6700) report POWR=1 the moment they accept "on", before the
                # lamp strikes. Where LAMP ? is supported, also wait for a lamp-on flag.
                if target == "1" and self.via == "pjlink" and not self._lamp_lit():
                    if "lamp starting" not in seen:
                        seen.append("lamp starting")
                else:
                    return {"reached": POWER_STATES[target], "states_seen": seen}
            if time.monotonic() > deadline:
                raise PJError(f"did not reach {POWER_STATES[target]} in time; states seen: {seen}")
            time.sleep(POLL_INTERVAL)

    def _lamp_lit(self) -> bool:
        try:
            flags = self.pjlink.lamp().split()[1::2]
        except PJLinkError as e:
            return e.code != "ERR3"  # ERR1 = no lamp report: trust POWR; ERR3 = still starting
        except (PJError, ConnectionRefusedError):
            return False
        return "1" in flags or not flags

    def status(self) -> dict:
        out: dict = {"unit": self.key, "name": self.name, "ip": self.ip}

        def q(label, fn, fmt=lambda v: v):
            try:
                out[label] = fmt(fn())
            except PJLinkError as e:
                if e.code == "ERR1":
                    out[label] = "n/a (not supported)"
                elif e.code == "ERR3" and out.get("power") != "on":
                    out[label] = f"- ({out.get('power')})"  # no input reported in standby/transition
                else:
                    out[label] = f"error: {e}"
            except (PJError, ConnectionRefusedError) as e:
                out[label] = f"error: {e}"

        try:
            out["power"] = POWER_STATES.get(s := self.power_state(time.monotonic() + 20), s)
            out["via"] = self.via
        except (PJError, ConnectionRefusedError) as e:
            out["power"] = f"error: {e}"
            out["ok"] = False
            return out
        if self.via == "pjlink":
            q("input", self.pjlink.input, fmt_input)
            q("hours", self.pjlink.lamp, fmt_lamp)
            q("errors", self.pjlink.erst, fmt_erst)
        out["ok"] = True
        return out


def fmt_input(v: str) -> str:
    if len(v) == 2 and v[0] in INPUT_TYPES:
        return f"{INPUT_TYPES[v[0]]} {v[1]} ({v})"
    return v


def fmt_lamp(v: str) -> str:
    parts = v.split()
    lamps = []
    for i in range(0, len(parts) - 1, 2):
        lamps.append(f"lamp{i // 2 + 1}: {parts[i]} h ({'on' if parts[i + 1] == '1' else 'off'})")
    return ", ".join(lamps) or v


def fmt_erst(v: str) -> str:
    if len(v) != 6:
        return v
    bad = [f"{f}={ERST_LEVELS.get(c, c)}" for f, c in zip(ERST_FIELDS, v) if c != "0"]
    return "no errors" if not bad else ", ".join(bad)


log = logging.getLogger("pj")


def units_for(cfg: dict, target: str) -> list[Unit]:
    projs = cfg["projectors"]
    if target == "all":
        return [Unit(k, v) for k, v in projs.items()]
    if target not in projs:
        raise ConfigError(f"unknown target '{target}'. Known: all, {', '.join(projs)}")
    return [Unit(target, projs[target])]


def run_on(units: list[Unit], fn) -> list[tuple[Unit, object, Exception | None]]:
    """Run fn(unit) concurrently; return (unit, result, error)."""
    out = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(units))) as ex:
        futs = {ex.submit(fn, u): u for u in units}
        for f, u in futs.items():
            try:
                out.append((u, f.result(), None))
            except (PJError, ConnectionRefusedError, OSError) as e:
                out.append((u, None, e))
    return out


# --------------------------------------------------------------------------- discovery

def _port_open(ip: str, port: int, timeout: float = 0.6) -> bool:
    try:
        with socket.create_connection((ip, port), timeout=timeout):
            return True
    except OSError:
        return False


def _ethernet_neighbors(subnet: str) -> set[str]:
    """IPv4 neighbours (ARP) on the projector interface, including link-local/DHCP addresses."""
    found: set[str] = set()
    try:
        out = subprocess.run(["arp", "-a"], capture_output=True, text=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return found
    section_ok = False
    net = ipaddress.ip_network(subnet, strict=False)
    for line in out.splitlines():
        m = re.match(r"Interface:\s+(\S+)", line)
        if m:
            section_ok = ipaddress.ip_address(m.group(1)) in net
            continue
        m = re.match(r"\s+(\d+\.\d+\.\d+\.\d+)\s+([0-9a-f-]{17})\s+dynamic", line, re.I)
        if m and section_ok:
            found.add(m.group(1))
    return found


OUI_VENDORS = {"045d4b": "sony", "080046": "sony", "001d0d": "sony", "00809f": "panasonic",
               "0080f0": "panasonic", "0050e1": "panasonic", "8c5ab0": "panasonic"}


def dhcp_clients(listen_ip: str, seconds: float = 15.0) -> list[dict]:
    """Passively listen for DHCP DISCOVER/REQUEST on the projector interface.

    Finds units stuck in DHCP mode (no DHCP server on this LAN), which otherwise never
    appear in an IP scan. Nothing is ever sent.
    """
    seen: dict[str, dict] = {}
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind((listen_ip, 67))
    except OSError as e:
        log.warning("DHCP listen on %s:67 unavailable: %s", listen_ip, e)
        return []
    end = time.monotonic() + seconds
    try:
        while (left := end - time.monotonic()) > 0:
            s.settimeout(left)
            try:
                pkt, _ = s.recvfrom(2048)
            except socket.timeout:
                break
            if len(pkt) < 240 or pkt[0] != 1 or pkt[236:240] != b"\x63\x82\x53\x63":
                continue
            mac = pkt[28:34].hex()
            info = seen.setdefault(mac, {"mac": "-".join(mac[i:i + 2] for i in range(0, 12, 2)).upper(),
                                         "vendor": OUI_VENDORS.get(mac[:6], "unknown"), "count": 0})
            info["count"] += 1
            i = 240
            while i + 2 <= len(pkt) and pkt[i] != 255:
                if pkt[i] == 0:
                    i += 1
                    continue
                code, ln = pkt[i], pkt[i + 1]
                val = pkt[i + 2:i + 2 + ln]
                if code == 12:
                    info["hostname"] = val.decode("ascii", "replace")
                elif code == 60:
                    info["vendor_class"] = val.decode("ascii", "replace")
                    vl = info["vendor_class"].lower()
                    info["vendor"] = "sony" if "sony" in vl else "panasonic" if "panasonic" in vl else info["vendor"]
                elif code == 50 and ln == 4:
                    info["requested_ip"] = ".".join(map(str, val))
                i += 2 + ln
    finally:
        s.close()
    return list(seen.values())


def discover(cfg: dict, subnet: str = "192.168.50.0/24") -> list[dict]:
    net = ipaddress.ip_network(subnet)
    candidates = {str(h) for h in net.hosts()} | _ethernet_neighbors(subnet)
    local = {cfg.get("network", {}).get("ethernet_ip", "192.168.50.1")}
    candidates -= local
    with concurrent.futures.ThreadPoolExecutor(max_workers=64) as ex:
        open_hosts = [ip for ip, ok in zip(sorted(candidates), ex.map(lambda i: _port_open(i, PJLINK_PORT), sorted(candidates))) if ok]
    passwords = [""] + [p.get("pjlink_password", "") for p in cfg["projectors"].values()]
    results = []
    for ip in open_hosts:
        info: dict = {"ip": ip}
        for pw in dict.fromkeys(passwords):
            pj = PJLink(ip, pw)
            try:
                info["name"] = pj.name()
                info["inf1"] = pj.inf1()
                info["inf2"] = pj.inf2()
                info.pop("error", None)
                break
            except PJLinkError as e:
                info["error"] = str(e)
                if e.code != "ERRA":
                    break
            except (PJError, ConnectionRefusedError) as e:
                info["error"] = str(e)
                break
        blob = " ".join(str(info.get(k, "")) for k in ("name", "inf1", "inf2")).lower()
        info["vendor"] = "panasonic" if "panasonic" in blob else "sony" if "sony" in blob else "unknown"
        results.append(info)
    return results


MANUAL_STEPS = {
    "panasonic": "Panasonic PT-DZ6700: MENU > NETWORK > LAN SETTINGS (Wired LAN): DHCP = OFF, "
                 "IP ADDRESS = 192.168.50.10, SUBNET MASK = 255.255.255.0, (gateway blank) > STORE.",
    "sony": "Sony VPL-PHZ10: MENU > Connection/Power > Network Settings: DHCP = Off, "
            "IP Address = 192.168.50.11, Subnet Mask = 255.255.255.0 > Apply.",
}


def check_discovery(cfg: dict, found: list[dict]) -> list[str]:
    """Return problems: missing projectors or wrong IPs."""
    problems = []
    for key, p in cfg["projectors"].items():
        vendor = p.get("vendor", key)
        at_expected = [f for f in found if f["ip"] == p["ip"]]
        same_vendor = [f for f in found if f["vendor"] == vendor]
        if at_expected and at_expected[0]["vendor"] in (vendor, "unknown"):
            continue
        if same_vendor:
            problems.append(f"{p['name']} found at {same_vendor[0]['ip']} but expected {p['ip']}.\n  -> {MANUAL_STEPS.get(vendor, '')}")
        else:
            problems.append(f"{p['name']} not found (expected at {p['ip']}).\n  -> {MANUAL_STEPS.get(vendor, '')}")
    return problems


# --------------------------------------------------------------------------- CLI

def _print_status(u: Unit, st: dict | None, err: Exception | None) -> None:
    print(f"== {u.name} [{u.key}] {u.ip}")
    if err:
        print(f"   ERROR: {err}")
        return
    for k in ("power", "input", "hours", "errors", "via"):
        if k in st:
            print(f"   {k:<7}: {st[k]}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="pj", description="Projector power control (PJLink / NTCONTROL)")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--config", default=str(CONFIG_PATH))
    sub = ap.add_subparsers(dest="cmd", required=True)
    for c in ("on", "off", "status"):
        sp = sub.add_parser(c)
        sp.add_argument("target", nargs="?", default="all")
        if c != "status":
            sp.add_argument("--wait", action="store_true", help="poll until the target state is reached")
    sp = sub.add_parser("discover", help="scan 192.168.50.0/24 for PJLink hosts")
    sp.add_argument("--subnet", default="192.168.50.0/24")
    sp = sub.add_parser("raw", help="send a raw PJLink command, e.g. pj raw sony '%%1NAME ?'")
    sp.add_argument("target")
    sp.add_argument("command")
    args = ap.parse_args(argv)
    logging.basicConfig(level=logging.INFO if args.verbose else logging.WARNING,
                        format="%(levelname)s %(message)s")
    try:
        cfg = load_config(Path(args.config))
        src = f"cli:{getpass.getuser()}"
        if args.cmd == "discover":
            found = discover(cfg, args.subnet)
            for f in found:
                print(f"{f['ip']:<16} vendor={f['vendor']:<10} NAME={f.get('name', '?')!r} "
                      f"INF1={f.get('inf1', '?')!r} INF2={f.get('inf2', '?')!r}"
                      + (f"  ({f['error']})" if "error" in f else ""))
            if not found:
                print("No hosts with TCP 4352 open.")
            problems = check_discovery(cfg, found)
            if problems:
                eth_ip = cfg.get("network", {}).get("ethernet_ip", "192.168.50.1")
                print(f"Listening 15 s for DHCP clients on {eth_ip} (units left in DHCP mode)...")
                for c in dhcp_clients(eth_ip):
                    print(f"DHCP client {c['mac']} vendor={c['vendor']} hostname={c.get('hostname', '-')!r} "
                          f"class={c.get('vendor_class', '-')!r} requested={c.get('requested_ip', '-')} "
                          f"({c['count']} requests, no DHCP server here -> set a static IP on the unit)")
            for p in problems:
                print("PROBLEM:", p)
            return 1 if problems else 0
        units = units_for(cfg, args.target)
        if args.cmd == "raw":
            val = units[0].pjlink.raw(args.command)
            print(val)
            return 0
        if args.cmd == "status":
            res = run_on(units, lambda u: u.status())
            for u, st, err in res:
                _print_status(u, st, err)
            return 0 if all(e is None and st.get("ok") for _, st, e in res) else 1
        on = args.cmd == "on"
        res = run_on(units, lambda u: u.set_power(on, wait=args.wait))
        rc = 0
        for u, r, err in res:
            if err:
                rc = 1
                audit(src, u.key, args.cmd, f"FAIL {err}")
                print(f"{u.name}: FAILED - {err}")
            else:
                audit(src, u.key, args.cmd, f"OK {json.dumps(r)}")
                verb = "command sent" if r["sent"] else f"already {r['previous']}, nothing sent"
                extra = f", reached {r['reached']} in {r['elapsed_s']}s" if "reached" in r else ""
                print(f"{u.name}: {verb} (via {r['via']}){extra}")
        return rc
    except ConfigError as e:
        print(f"config error: {e}", file=sys.stderr)
        return 2
    except (PJError, ConnectionRefusedError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
