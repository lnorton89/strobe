"""Offline tests for pjctl against a fake PJLink Class 1 projector on 127.0.0.1.

Covers: auth digest, ERRA, ERR3 retry, waiting out warming/cooling before a power command,
no-op when already in the requested state, connection refused, and error-code messages.

    python tools/projector-control/tests/test_pjctl.py      (exit 0 = all pass)
"""
import hashlib
import socketserver
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "app"))
import pjctl  # noqa: E402

pjctl.POLL_INTERVAL = 0.2
PASSWORD = "secret"
STATE = {"power": "0", "busy_until": 0.0, "err3_left": 0}


class FakeProjector(socketserver.StreamRequestHandler):
    def handle(self):
        rnd = "498e4a67"
        self.wfile.write(f"PJLINK 1 {rnd}\r".encode())
        line = b""
        while not line.endswith(b"\r"):
            c = self.rfile.read(1)
            if not c:
                return
            line += c
        line = line.decode().strip()
        digest, cmd = line[:32], line[32:]
        if digest != hashlib.md5((rnd + PASSWORD).encode()).hexdigest():
            self.wfile.write(b"PJLINK ERRA\r")
            return
        now = time.monotonic()
        if STATE["busy_until"] <= now and STATE["power"] in "23":
            STATE["power"] = "1" if STATE["power"] == "3" else "0"
        head, arg = cmd[:6], cmd[7:]
        if head == "%1POWR" and arg == "?":
            r = STATE["power"]
        elif head == "%1POWR":
            if STATE["err3_left"]:
                STATE["err3_left"] -= 1
                r = "ERR3"
            elif STATE["power"] in "23":
                STATE["violation"] = True  # a power command arrived mid-transition
                r = "ERR3"
            else:
                STATE["power"] = "3" if arg == "1" else "2"
                STATE["busy_until"] = now + 1.0
                r = "OK"
        elif head == "%1LAMP":
            r = "1234 1" if STATE["power"] == "1" else "1234 0"
        elif head == "%1ERST":
            r = "000010"
        elif head == "%1INPT":
            r = "31"
        elif head == "%1NAME":
            r = "FAKE"
        else:
            r = "ERR1"
        self.wfile.write(f"{head}={r}\r".encode())


def main() -> int:
    srv = socketserver.ThreadingTCPServer(("127.0.0.1", 0), FakeProjector)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    def unit(pw=PASSWORD):
        u = pjctl.Unit("fake", {"name": "Fake", "ip": "127.0.0.1", "protocol": "pjlink"})
        u.pjlink = pjctl.PJLink("127.0.0.1", pw, port)
        return u

    fails = 0

    def check(name, cond, detail=""):
        nonlocal fails
        print(("PASS " if cond else "FAIL ") + name, detail)
        fails += 0 if cond else 1

    st = unit().status()
    check("status fields", st["power"] == "off" and "1234" in st["hours"]
          and "filter=WARNING" in st["errors"] and "DIGITAL" in st["input"], st)
    try:
        unit("wrong").power_state()
        check("ERRA raised", False)
    except pjctl.PJLinkError as e:
        check("ERRA raised", e.code == "ERRA", str(e))

    STATE["err3_left"] = 2
    r = unit().set_power(True, wait=True, wait_timeout=20)
    check("on with 2x ERR3 retry + wait", r["reached"] == "on" and "warming" in r["states_seen"], r)
    r = unit().set_power(True)
    check("on when already on = no-op", r["sent"] is False, r)

    unit().pjlink.set_power(False)  # start cooling directly
    r = unit().set_power(True, wait=True, wait_timeout=20)
    check("waits out cooling before sending on", r["reached"] == "on", r)
    check("never commanded mid-transition", not STATE.get("violation"))

    try:
        pjctl.PJLink("127.0.0.1", "", 1).power()
        check("refused -> ConnectionRefusedError", False)
    except ConnectionRefusedError:
        check("refused -> ConnectionRefusedError", True)
    for code in ("ERRA", "ERR1", "ERR2", "ERR3", "ERR4"):
        msg = str(pjctl.PJLinkError(code, "%1POWR"))
        check(f"message {code}", code in msg and "unknown" not in msg, msg)

    srv.shutdown()
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
