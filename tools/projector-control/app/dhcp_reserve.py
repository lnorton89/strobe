"""Minimal DHCP responder for the projector LAN: answers ONLY reserved MACs.

Bound to the Ethernet IP (192.168.50.1:67). Offers the reserved IP, /24 mask, no router,
no DNS, so nothing can steal the PC's default route. Ignores every other client.
Usage: python dhcp_reserve.py [--seconds N]   (default: run until stopped)
"""
from __future__ import annotations

import argparse
import ipaddress
import logging
import socket
import struct
import sys
import time

import pjctl

MAGIC = b"\x63\x82\x53\x63"
LEASE_S = 86400


def reservations(cfg: dict) -> dict[bytes, str]:
    out = {}
    for p in cfg["projectors"].values():
        mac = p.get("mac")
        if mac:
            out[bytes.fromhex(mac.replace("-", "").replace(":", ""))] = p["ip"]
    return out


def build(reply_type: int, req: bytes, yiaddr: str, server: str) -> bytes:
    xid, flags, chaddr = req[4:8], req[10:12], req[28:44]
    hdr = struct.pack("!BBBB", 2, 1, 6, 0) + xid + b"\x00\x00" + flags
    hdr += b"\x00" * 4 + socket.inet_aton(yiaddr) + socket.inet_aton(server) + b"\x00" * 4
    hdr += chaddr + b"\x00" * 192 + MAGIC
    opts = bytes([53, 1, reply_type]) + bytes([54, 4]) + socket.inet_aton(server)
    opts += bytes([51, 4]) + struct.pack("!I", LEASE_S)
    opts += bytes([1, 4]) + socket.inet_aton("255.255.255.0") + b"\xff"
    return hdr + opts


def msg_type(pkt: bytes) -> int | None:
    i = 240
    while i + 1 < len(pkt) and pkt[i] != 255:
        if pkt[i] == 0:
            i += 1
            continue
        if pkt[i] == 53:
            return pkt[i + 2]
        i += 2 + pkt[i + 1]
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seconds", type=float, default=0)
    args = ap.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
    cfg = pjctl.load_config()
    server = cfg.get("network", {}).get("ethernet_ip", "192.168.50.1")
    net = ipaddress.ip_network(cfg.get("network", {}).get("projector_subnet", "192.168.50.0/24"))
    res = {m: ip for m, ip in reservations(cfg).items() if ipaddress.ip_address(ip) in net}
    if not res:
        logging.error("no projector has a 'mac' in config.json; nothing to serve")
        return 2
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    s.bind((server, 67))
    logging.info("serving reservations on %s:67: %s", server, {m.hex("-"): ip for m, ip in res.items()})
    end = time.monotonic() + args.seconds if args.seconds else None
    acked = set()
    while end is None or time.monotonic() < end:
        s.settimeout(5 if end is None else max(0.1, end - time.monotonic()))
        try:
            pkt, _ = s.recvfrom(2048)
        except socket.timeout:
            continue
        if len(pkt) < 240 or pkt[0] != 1 or pkt[236:240] != MAGIC:
            continue
        mac = pkt[28:34]
        ip = res.get(mac)
        t = msg_type(pkt)
        if ip is None or t not in (1, 3):
            continue
        reply = 2 if t == 1 else 5          # DISCOVER->OFFER, REQUEST->ACK
        s.sendto(build(reply, pkt, ip, server), ("255.255.255.255", 68))
        logging.info("%s %s -> %s", "OFFER" if reply == 2 else "ACK", mac.hex("-"), ip)
        if reply == 5:
            acked.add(mac)
            pjctl.audit("dhcp", mac.hex("-"), "lease", f"ACK {ip}")
    return 0 if acked else 1


if __name__ == "__main__":
    sys.exit(main())
