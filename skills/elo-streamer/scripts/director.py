#!/usr/bin/env python3
"""director.py — switch OBS scenes by itself while an agent is on stream.

    python3 director.py --agent 0xAGENT
    python3 director.py --agent 0xAGENT --obs ws://127.0.0.1:4455 --every 30
    python3 director.py --agent 0xAGENT --dry-run        # print, touch no OBS

Every `--every` seconds it reads the island floor (`/api/agents/floor`, the
same read `gates(what='floor')` makes) and moves OBS between two scenes of
the stream kit's collection: the agent's scene while the agent is on the
island ("Watch an agent"), and "Be right back" while it is not. It only ever
moves between those two: a person who cuts to another scene is left there.
A state has to read the same twice in a row before it moves, so one late
answer never flips the stream, and a read that fails moves nothing.

OBS side: Tools → WebSocket Server Settings → enable it (OBS 28 and later
have it built in). If it has a password, put it in the environment as
`OBS_WEBSOCKET_PASSWORD` or in a file named by `--password-file`, readable by
you alone. The password is never taken on the command line, where any user on
the machine can read it.

Standard library only: obs-websocket v5 over a small RFC 6455 client, and
urllib for the floor. It sends nothing anywhere but the one GET and the OBS
socket, and it holds no wallet key: the agent's wallet is only a name to look
up.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import socket
import stat
import struct
import sys
import time
import urllib.request
from urllib.parse import urlsplit

FLOOR = "https://elopros.com/api/agents/floor"
ON_SCENE = "Watch an agent"
OFF_SCENE = "Be right back"
WALLET = re.compile(r"^0x[0-9a-fA-F]{40}$")
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


# ── the decision ────────────────────────────────────────────────────────────
def decide(current: str | None, on_island: bool | None, streak: int,
           on: str = ON_SCENE, off: str = OFF_SCENE, need: int = 2) -> str | None:
    """The scene to switch to, or None to stay.

    `on_island` is None when the floor could not be read: nothing moves.
    Only the two managed scenes are ever left: any other scene is a person's.
    `streak` is how many reads in a row said the same."""
    if on_island is None or streak < need or current not in (on, off):
        return None
    want = on if on_island else off
    return want if want != current else None


def on_island(floor: dict | None, agent: str) -> bool | None:
    """True or False off a floor read; None when the read failed."""
    if not isinstance(floor, dict) or not isinstance(floor.get("agents"), list):
        return None
    if not (floor.get("roster") or {}).get("reachable", True):
        return None
    row = next((r for r in floor["agents"] if isinstance(r, dict) and r.get("wallet") == agent), None)
    return bool(row and row.get("on_island"))


def read_floor(url: str = FLOOR, timeout: float = 10.0) -> dict | None:
    try:
        req = urllib.request.Request(url, headers={"Accept": "application/json",
                                                   "User-Agent": "elo-streamer-director"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read(4 << 20))
    except Exception:  # noqa: BLE001 — a failed read moves nothing
        return None


# ── obs-websocket v5 ────────────────────────────────────────────────────────
def obs_auth(password: str, salt: str, challenge: str) -> str:
    """base64(sha256(base64(sha256(password + salt)) + challenge)), per the
    obs-websocket v5 protocol."""
    secret = base64.b64encode(hashlib.sha256((password + salt).encode()).digest()).decode()
    return base64.b64encode(hashlib.sha256((secret + challenge).encode()).digest()).decode()


def accept_key(key: str) -> str:
    return base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()


def frame(payload: bytes, opcode: int = 1, mask: bytes | None = None) -> bytes:
    """One client frame, FIN set and masked (RFC 6455 §5.3: a client always masks)."""
    mask = mask if mask is not None else os.urandom(4)
    n = len(payload)
    head = bytes([0x80 | opcode])
    if n < 126:
        head += bytes([0x80 | n])
    elif n < 1 << 16:
        head += bytes([0x80 | 126]) + struct.pack("!H", n)
    else:
        head += bytes([0x80 | 127]) + struct.pack("!Q", n)
    return head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload))


def unframe(read) -> tuple[int, bytes]:
    """(opcode, payload) of one whole message, joining continuations. `read(n)`
    returns exactly n bytes."""
    parts, first = [], None
    while True:
        b0, b1 = read(2)
        op, fin = b0 & 0x0F, b0 & 0x80
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack("!H", read(2))[0]
        elif n == 127:
            n = struct.unpack("!Q", read(8))[0]
        mask = read(4) if b1 & 0x80 else None
        data = read(n) if n else b""
        if mask:
            data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        if op >= 8:            # a control frame may arrive between fragments
            return op, data
        if first is None:
            first = op
        parts.append(data)
        if fin:
            return first, b"".join(parts)


class Obs:
    """A minimal obs-websocket v5 client: identify, then request and wait."""

    def __init__(self, url: str, password: str = "", timeout: float = 10.0):
        u = urlsplit(url)
        if u.scheme != "ws":
            raise ValueError("only ws:// (OBS listens on the machine it runs on)")
        self.sock = socket.create_connection((u.hostname or "127.0.0.1", u.port or 4455), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {u.path or '/'} HTTP/1.1\r\nHost: {u.hostname}:{u.port or 4455}\r\n"
                           f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                           f"Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(1)
            if not chunk:
                raise ConnectionError("OBS closed the connection during the handshake")
            head += chunk
        if b" 101 " not in head.split(b"\r\n", 1)[0] or accept_key(key).encode() not in head:
            raise ConnectionError("OBS refused the WebSocket handshake")
        self.n = 0
        hello = self.recv()
        d = hello.get("d") or {}
        ident = {"rpcVersion": 1, "eventSubscriptions": 0}
        auth = d.get("authentication")
        if auth:
            if not password:
                raise PermissionError("OBS wants a password: set OBS_WEBSOCKET_PASSWORD or --password-file")
            ident["authentication"] = obs_auth(password, auth["salt"], auth["challenge"])
        self.send({"op": 1, "d": ident})
        if self.recv().get("op") != 2:
            raise PermissionError("OBS did not accept the identify (wrong password?)")

    def _read(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("OBS closed the connection")
            buf += chunk
        return buf

    def send(self, msg: dict) -> None:
        self.sock.sendall(frame(json.dumps(msg).encode()))

    def recv(self) -> dict:
        while True:
            op, data = unframe(self._read)
            if op == 9:
                self.sock.sendall(frame(data, opcode=10))
            elif op == 8:
                raise ConnectionError("OBS closed the connection")
            elif op == 1:
                return json.loads(data)

    def request(self, kind: str, data: dict | None = None) -> dict:
        self.n += 1
        rid = f"director-{self.n}"
        self.send({"op": 6, "d": {"requestType": kind, "requestId": rid, "requestData": data or {}}})
        while True:
            msg = self.recv()
            d = msg.get("d") or {}
            if msg.get("op") == 7 and d.get("requestId") == rid:
                if not (d.get("requestStatus") or {}).get("result"):
                    raise RuntimeError(f"OBS refused {kind}: {(d.get('requestStatus') or {}).get('comment')}")
                return d.get("responseData") or {}

    def scene(self) -> str | None:
        r = self.request("GetCurrentProgramScene")
        return r.get("currentProgramSceneName") or r.get("sceneName")

    def cut(self, name: str) -> None:
        self.request("SetCurrentProgramScene", {"sceneName": name})


def password(path: str | None) -> str:
    if path:
        p = os.path.expanduser(path)
        if os.stat(p).st_mode & (stat.S_IRWXG | stat.S_IRWXO):
            sys.exit(f"{path} is readable by others: chmod 600 it first")
        with open(p, encoding="utf-8") as f:
            return f.read().strip()
    return os.environ.get("OBS_WEBSOCKET_PASSWORD", "")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Switch OBS scenes while an agent is on stream.")
    ap.add_argument("--agent", required=True, help="the agent's wallet, 0x + 40 hex")
    ap.add_argument("--obs", default="ws://127.0.0.1:4455", help="OBS's WebSocket server")
    ap.add_argument("--password-file", help="a file holding OBS's WebSocket password (mode 600)")
    ap.add_argument("--on", default=ON_SCENE, help=f"the scene while it plays ({ON_SCENE!r})")
    ap.add_argument("--off", default=OFF_SCENE, help=f"the scene while it is away ({OFF_SCENE!r})")
    ap.add_argument("--every", type=float, default=30.0, help="seconds between reads (30, at least 10)")
    ap.add_argument("--floor", default=FLOOR, help=argparse.SUPPRESS)
    ap.add_argument("--dry-run", action="store_true", help="print what it would do; touch no OBS")
    ap.add_argument("--once", action="store_true", help="one read, then stop")
    a = ap.parse_args(argv)
    if not WALLET.match(a.agent):
        sys.exit("--agent is a wallet: 0x followed by 40 hex digits")
    agent, every = a.agent.lower(), max(10.0, a.every)
    obs = None if a.dry_run else Obs(a.obs, password(a.password_file))
    sim = a.off                 # a dry run starts on the away scene and pretends to cut
    last, streak = None, 0
    while True:
        here = on_island(read_floor(a.floor), agent)
        streak = streak + 1 if here is not None and here == last else (1 if here is not None else 0)
        last = here if here is not None else last
        current = (obs.scene() if obs else sim) if here is not None else None
        target = decide(current, here, streak, a.on, a.off, need=1 if a.once else 2)
        stamp = time.strftime("%H:%M:%S")
        if here is None:
            print(f"{stamp} couldn't read the floor; staying put", flush=True)
        elif target:
            if obs:
                obs.cut(target)
            else:
                sim = target
            print(f"{stamp} {'on' if here else 'off'} the island → {target}"
                  + (" (dry run)" if not obs else ""), flush=True)
        else:
            print(f"{stamp} {'on' if here else 'off'} the island · {current or 'no scene'}", flush=True)
        if a.once:
            return 0
        time.sleep(every)


if __name__ == "__main__":
    sys.exit(main())
