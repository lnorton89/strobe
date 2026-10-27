"""Token-protected HTTP API + single-page UI for projector power.

Binds ONLY to the Tailscale IPv4 (if Tailscale is installed and up) or the Wi-Fi IPv4.
Never 0.0.0.0. Rebinds automatically if that address changes (DHCP renew).
"""
from __future__ import annotations

import hmac
import http.cookies
import http.server
import ipaddress
import json
import logging
import logging.handlers
import re
import shutil
import socket
import socketserver
import subprocess
import sys
import threading
import time
import urllib.parse
from pathlib import Path

import pjctl

HERE = Path(__file__).resolve().parent
UI_FILE = HERE / "ui.html"
COOKIE = "pj_token"

log = logging.getLogger("pj.server")


# --------------------------------------------------------------------------- bind address

def _ps(cmd: str) -> str:
    try:
        r = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", cmd],
                           capture_output=True, text=True, timeout=20)
        return r.stdout.strip()
    except (OSError, subprocess.SubprocessError) as e:
        log.warning("powershell query failed: %s", e)
        return ""


def tailscale_ip() -> str | None:
    exe = shutil.which("tailscale") or (
        r"C:\Program Files\Tailscale\tailscale.exe" if Path(r"C:\Program Files\Tailscale\tailscale.exe").exists() else None)
    if not exe:
        return None
    try:
        r = subprocess.run([exe, "ip", "-4"], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    for line in r.stdout.split():
        try:
            if ipaddress.ip_address(line) in ipaddress.ip_network("100.64.0.0/10"):
                return line
        except ValueError:
            continue
    return None


def wifi_ip(alias: str) -> str | None:
    out = _ps(f"(Get-NetIPAddress -InterfaceAlias '{alias}' -AddressFamily IPv4 -ErrorAction SilentlyContinue "
              f"| Where-Object {{ $_.AddressState -eq 'Preferred' -and $_.IPAddress -notlike '169.254.*' }} "
              f"| Select-Object -First 1).IPAddress")
    try:
        return str(ipaddress.ip_address(out)) if out else None
    except ValueError:
        return None


def resolve_bind(cfg: dict) -> tuple[str | None, str]:
    api = cfg.get("api", {})
    mode = api.get("bind", "auto")
    if mode not in ("auto", "tailscale", "wifi"):
        raise pjctl.ConfigError("api.bind must be auto, tailscale or wifi (explicit IPs are not allowed)")
    if mode in ("auto", "tailscale"):
        ts = tailscale_ip()
        if ts:
            return ts, "tailscale"
        if mode == "tailscale":
            return None, "tailscale"
    return wifi_ip(cfg.get("network", {}).get("wifi_alias", "Wi-Fi")), "wifi"


def safe_bind(ip: str, cfg: dict) -> bool:
    a = ipaddress.ip_address(ip)
    proj_net = ipaddress.ip_network(cfg.get("network", {}).get("projector_subnet", "192.168.50.0/24"))
    return not (a.is_unspecified or a.is_loopback or a.is_link_local or a in proj_net)


# --------------------------------------------------------------------------- HTTP

class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "pjctl"
    sys_version = ""
    cfg: dict = {}
    token: str = ""

    def log_message(self, fmt, *args):  # access log -> server log
        log.info("%s %s", self.client_address[0], fmt % args)

    # -- helpers
    def _send(self, code: int, body: bytes, ctype: str = "application/json", headers: dict | None = None):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Referrer-Policy", "no-referrer")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def _json(self, code: int, obj) -> None:
        self._send(code, json.dumps(obj).encode())

    def _presented_token(self, query: dict) -> str | None:
        auth = self.headers.get("Authorization", "")
        if auth.startswith("Bearer "):
            return auth[7:].strip()
        if "token" in query:
            return query["token"][0]
        c = http.cookies.SimpleCookie(self.headers.get("Cookie", ""))
        if COOKIE in c:
            return c[COOKIE].value
        return None

    def _authorized(self, query: dict) -> bool:
        t = self._presented_token(query)
        return bool(t) and hmac.compare_digest(t.encode(), self.token.encode())

    def _reject(self, html: bool) -> None:
        pjctl.audit(self.client_address[0], "-", f"{self.command} {self.path.split('?')[0]}", "DENIED no/invalid token")
        if html:
            body = (b"<!doctype html><meta name=viewport content='width=device-width'>"
                    b"<body style='background:#111;color:#ddd;font:16px system-ui;padding:2em'>"
                    b"<h2>Projector control</h2><form method=get action='/'>"
                    b"<input name=token type=password placeholder='API token' autofocus "
                    b"style='padding:.6em;background:#222;color:#eee;border:1px solid #444'> "
                    b"<button style='padding:.6em'>Unlock</button></form>")
            self._send(401, body, "text/html; charset=utf-8", {"WWW-Authenticate": "Bearer"})
        else:
            self._json(401, {"error": "unauthorized"})

    # -- routes
    def do_GET(self):
        url = urllib.parse.urlsplit(self.path)
        q = urllib.parse.parse_qs(url.query)
        if not self._authorized(q):
            return self._reject(html=not url.path.startswith("/api"))
        if url.path == "/":
            if "token" in q:  # exchange query token for a cookie, then drop it from the URL
                return self._send(303, b"", headers={
                    "Location": "/",
                    "Set-Cookie": f"{COOKIE}={q['token'][0]}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000"})
            return self._send(200, UI_FILE.read_bytes(), "text/html; charset=utf-8")
        if url.path == "/api/units":
            return self._json(200, {k: {"name": v["name"], "ip": v["ip"]} for k, v in self.cfg["projectors"].items()})
        m = re.fullmatch(r"/api/status(?:/(\w+))?", url.path)
        if m:
            return self._status(m.group(1) or "all")
        self._json(404, {"error": "not found"})

    def do_POST(self):
        url = urllib.parse.urlsplit(self.path)
        if not self._authorized({}):
            return self._reject(html=False)
        m = re.fullmatch(r"/api/power/(\w+)/(on|off)", url.path)
        if not m:
            return self._json(404, {"error": "not found"})
        target, action = m.groups()
        src = self.client_address[0]
        try:
            units = pjctl.units_for(self.cfg, target)
        except pjctl.ConfigError as e:
            return self._json(404, {"error": str(e)})
        res = pjctl.run_on(units, lambda u: u.set_power(action == "on"))
        out, ok = {}, True
        for u, r, err in res:
            if err:
                ok = False
                out[u.key] = {"ok": False, "error": str(err)}
                pjctl.audit(src, u.key, action, f"FAIL {err}")
            else:
                out[u.key] = {"ok": True, **r}
                pjctl.audit(src, u.key, action, f"OK {json.dumps(r)}")
        self._json(200 if ok else 502, out)

    def _status(self, target: str):
        try:
            units = pjctl.units_for(self.cfg, target)
        except pjctl.ConfigError as e:
            return self._json(404, {"error": str(e)})
        res = pjctl.run_on(units, lambda u: u.status())
        out = {u.key: (st if err is None else {"ok": False, "error": str(err)}) for u, st, err in res}
        pjctl.audit(self.client_address[0], target, "status",
                    "OK" if all(v.get("ok") for v in out.values()) else "PARTIAL")
        self._json(200, out)


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = False


# --------------------------------------------------------------------------- main loop

def setup_logging() -> None:
    pjctl.LOG_DIR.mkdir(parents=True, exist_ok=True)
    h = logging.handlers.RotatingFileHandler(pjctl.LOG_DIR / "server.log", maxBytes=1_000_000,
                                             backupCount=5, encoding="utf-8")
    h.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s %(message)s"))
    root = logging.getLogger()
    root.setLevel(logging.INFO)
    root.addHandler(h)
    root.addHandler(logging.StreamHandler(sys.stderr))


def main() -> int:
    setup_logging()
    try:
        cfg = pjctl.load_config()
    except pjctl.ConfigError as e:
        log.error("%s", e)
        return 2
    token = cfg.get("api", {}).get("token", "")
    if len(token) < 24:
        log.error("api.token missing or shorter than 24 chars; refusing to start")
        return 2
    Handler.cfg, Handler.token = cfg, token
    port = int(cfg.get("api", {}).get("port", 8780))

    while True:
        try:
            ip, kind = resolve_bind(cfg)
        except pjctl.ConfigError as e:
            log.error("%s", e)
            return 2
        if not ip or not safe_bind(ip, cfg):
            log.warning("no usable %s address yet (%s); retrying in 15s", kind, ip)
            time.sleep(15)
            continue
        try:
            httpd = Server((ip, port), Handler)
        except OSError as e:
            log.warning("bind %s:%d failed: %s; retrying in 15s", ip, port, e)
            time.sleep(15)
            continue
        log.info("listening on http://%s:%d/ (%s)", ip, port, kind)

        def watch(bound=ip):
            while True:
                time.sleep(30)
                try:
                    now, _ = resolve_bind(cfg)
                except pjctl.ConfigError:
                    now = None
                if now != bound:
                    log.warning("bind address changed %s -> %s; rebinding", bound, now)
                    httpd.shutdown()
                    return

        threading.Thread(target=watch, daemon=True).start()
        httpd.serve_forever()
        httpd.server_close()


if __name__ == "__main__":
    sys.exit(main())
