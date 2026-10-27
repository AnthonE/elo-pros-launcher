#!/usr/bin/env python3
"""test_director.py — the scene director (director.py), offline.

A fake OBS speaks obs-websocket v5 over a real socket on loopback, so the
handshake, the password, the frames and the two requests are the ones a real
OBS sees. The floor is a dict.

    python3 test_director.py
"""
from __future__ import annotations

import base64
import hashlib
import json
import socket
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import director as D  # noqa: E402

passed = failed = 0


def check(label: str, ok: bool, detail: str = "") -> None:
    global passed, failed
    passed += ok
    failed += not ok
    print(("  ok   " if ok else "  FAIL ") + label + ("" if ok or not detail else f" — {detail}"))


A = "0x" + "ab" * 20

# the decision
check("on the island, on the away scene, twice → the agent's scene",
      D.decide("Be right back", True, 2) == "Watch an agent")
check("off the island, on the agent's scene, twice → be right back",
      D.decide("Watch an agent", False, 2) == "Be right back")
check("one read is not enough to move", D.decide("Be right back", True, 1) is None)
check("a failed read moves nothing", D.decide("Be right back", None, 5) is None)
check("a scene a person cut to is left alone", D.decide("Just chatting", False, 9) is None)
check("already right stays", D.decide("Watch an agent", True, 3) is None)
floor = {"roster": {"reachable": True}, "agents": [{"wallet": A, "on_island": True}]}
check("the floor: on", D.on_island(floor, A) is True)
check("the floor: an agent not listed is off", D.on_island(floor, "0x" + "cd" * 20) is False)
check("the floor: a dark roster is unknown, not off",
      D.on_island({"roster": {"reachable": False}, "agents": []}, A) is None)
check("the floor: no answer is unknown", D.on_island(None, A) is None)

# RFC 6455 §1.3's own example, and a frame round trip
check("the accept key matches RFC 6455's example",
      D.accept_key("dGhlIHNhbXBsZSBub25jZQ==") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
for n in (5, 300, 70000):
    raw = D.frame(b"x" * n, mask=b"\x01\x02\x03\x04")
    buf = memoryview(raw)
    pos = [0]

    def read(k):
        out = bytes(buf[pos[0]:pos[0] + k])
        pos[0] += k
        return out
    check(f"a {n}-byte frame round-trips", D.unframe(read) == (1, b"x" * n))

# a fake OBS on loopback
PW = "hunter2"
SALT, CHAL = "c2FsdA==", "Y2hhbGxlbmdl"
want_auth = D.obs_auth(PW, SALT, CHAL)
secret = base64.b64encode(hashlib.sha256((PW + SALT).encode()).digest()).decode()
check("the auth string is the v5 formula",
      want_auth == base64.b64encode(hashlib.sha256((secret + CHAL).encode()).digest()).decode())
seen = {"scene": "Be right back", "cuts": [], "auth": None}


def server(sock):
    c, _ = sock.accept()
    head = b""
    while b"\r\n\r\n" not in head:
        head += c.recv(1)
    key = [ln.split(b": ", 1)[1] for ln in head.split(b"\r\n") if ln.lower().startswith(b"sec-websocket-key")][0]
    c.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
              b"Sec-WebSocket-Accept: " + D.accept_key(key.decode()).encode() + b"\r\n\r\n")

    def send(msg):  # a server frame is unmasked
        data = json.dumps(msg).encode()
        c.sendall(bytes([0x81, len(data)]) if len(data) < 126 else bytes([0x81, 126]) + len(data).to_bytes(2, "big"))
        c.sendall(data)

    def read(k):
        buf = b""
        while len(buf) < k:
            chunk = c.recv(k - len(buf))
            if not chunk:
                raise ConnectionError
            buf += chunk
        return buf
    send({"op": 0, "d": {"rpcVersion": 1, "authentication": {"salt": SALT, "challenge": CHAL}}})
    c.sendall(bytes([0x89, 0]))  # a ping before identify: the client must pong and carry on
    try:
        while True:
            op, data = D.unframe(read)
            if op != 1:
                continue
            msg = json.loads(data)
            d = msg["d"]
            if msg["op"] == 1:
                seen["auth"] = d.get("authentication")
                send({"op": 2, "d": {"negotiatedRpcVersion": 1}})
            elif d["requestType"] == "GetCurrentProgramScene":
                send({"op": 7, "d": {"requestId": d["requestId"], "requestStatus": {"result": True},
                                     "responseData": {"currentProgramSceneName": seen["scene"]}}})
            elif d["requestType"] == "SetCurrentProgramScene":
                seen["scene"] = d["requestData"]["sceneName"]
                seen["cuts"].append(seen["scene"])
                send({"op": 7, "d": {"requestId": d["requestId"], "requestStatus": {"result": True}}})
    except ConnectionError:
        pass


ls = socket.socket()
ls.bind(("127.0.0.1", 0))
ls.listen(1)
threading.Thread(target=server, args=(ls,), daemon=True).start()
obs = D.Obs(f"ws://127.0.0.1:{ls.getsockname()[1]}", PW)
check("identified with the v5 auth string", seen["auth"] == want_auth, str(seen["auth"]))
check("reads the program scene", obs.scene() == "Be right back")
obs.cut("Watch an agent")
check("cuts to a scene", seen["cuts"] == ["Watch an agent"] and obs.scene() == "Watch an agent")
check("the password never rides the command line",
      "--password" not in D.__doc__.replace("--password-file", "") and "OBS_WEBSOCKET_PASSWORD" in D.__doc__)

print(f"\nall good — {passed} checks passed" if not failed else f"\nFAIL — {failed} of {passed + failed} checks failed")
sys.exit(1 if failed else 0)
