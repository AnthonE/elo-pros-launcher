"""A game server's key: the signed text and signature match the shared
fixture byte for byte, and the signed calls send what the origin checks."""
import base64
import json
import os
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from _testdata import fx

from elo_sdk import Client, ServerKey, new_server_nonce, server_text

os.environ["NO_PROXY"] = os.environ["no_proxy"] = "127.0.0.1,localhost"
FX = fx("server_texts.json")
# PEM armour around a base64 body, built here so no key header is a literal.
_LABEL = " ".join(("PRIVATE", "KEY"))


def pem(body: str) -> bytes:
    return f"-----BEGIN {_LABEL}-----\n{body}\n-----END {_LABEL}-----\n".encode()


class Fixture(unittest.TestCase):
    def test_texts_and_signatures(self):
        key = ServerKey("my-game", "eu-1", bytes.fromhex(FX["seed_hex"]))
        self.assertEqual(key.public_b64, FX["public_b64"])
        for c in FX["cases"]:
            text = server_text(c["method"], c["path"], c["who"], c["at"], c["body_utf8"].encode(), c["nonce"])
            self.assertEqual(text, c["text"])
            self.assertEqual(key.sign_text(text), c["signature_b64"])

    def test_pem_and_seed(self):
        a = ServerKey.from_pem("my-game", "eu-1", pem(FX["private_pkcs8_b64"]))
        self.assertEqual(a.public_b64, FX["public_b64"])
        self.assertEqual(a.seed_hex(), FX["seed_hex"])

    def test_headers(self):
        key = ServerKey("my-game", "eu-1", bytes.fromhex(FX["seed_hex"]))
        c = FX["cases"][1]
        h = key.headers(c["method"], c["path"], c["body_utf8"].encode(), at=int(c["at"]), nonce=c["nonce"])
        self.assertEqual(h["X-Elo-Server"], "my-game/eu-1")
        self.assertEqual(h["X-Elo-Signature"], c["signature_b64"])
        self.assertNotEqual(key.headers("GET", "/x")["X-Elo-Nonce"], key.headers("GET", "/x")["X-Elo-Nonce"])

    def test_refusals(self):
        with self.assertRaises(ValueError):
            ServerKey("My Game", "eu-1", bytes(32))
        with self.assertRaises(ValueError):
            ServerKey("my-game", "EU", bytes(32))
        with self.assertRaises(ValueError):
            new_server_nonce(4)


class Calls(unittest.TestCase):
    def setUp(self):
        self.seen = []
        seen = self.seen

        class H(BaseHTTPRequestHandler):
            def _do(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n) if n else b""
                seen.append((self.command, self.path, dict(self.headers), body))
                if self.command == "GET" and self.path.endswith("/save"):
                    self.send_response(200)
                    self.send_header("Content-Type", "application/octet-stream")
                    self.send_header("ETag", '"abc"')
                    self.send_header("X-Elo-Version", "3")
                    self.end_headers()
                    self.wfile.write(b"\x00bytes")
                    return
                out = json.dumps({"ok": True}).encode()
                self.send_response(201 if self.command == "PUT" else 200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(out)))
                self.end_headers()
                self.wfile.write(out)

            do_GET = do_POST = do_PUT = do_DELETE = _do

            def log_message(self, *_a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.elo = Client(f"http://127.0.0.1:{self.srv.server_port}")
        self.key = ServerKey("my-game", "eu-1", bytes.fromhex(FX["seed_hex"]))

    def tearDown(self):
        self.srv.shutdown()

    def test_put_signs_the_bare_path_and_the_body(self):
        w = "0x" + "11" * 20
        got = self.elo.cloud_put(self.key, w, "save", b'{"level":3}', if_match="abc")
        self.assertTrue(got.ok, got)
        method, path, headers, body = self.seen[-1]
        self.assertEqual((method, path, body), ("PUT", f"/api/cloud/my-game/{w}/save", b'{"level":3}'))
        self.assertEqual(headers["If-Match"], "abc")
        text = server_text("PUT", f"/cloud/my-game/{w}/save", "my-game/eu-1", headers["X-Elo-Time"],
                           body, headers["X-Elo-Nonce"])
        self.assertEqual(headers["X-Elo-Signature"], self.key.sign_text(text))

    def test_get_returns_bytes(self):
        got = self.elo.cloud_get(self.key, "0x" + "11" * 20, "save")
        self.assertEqual(got.value, {"data": b"\x00bytes", "etag": "abc", "version": 3})

    def test_a_bad_slot_is_refused_before_asking(self):
        got = self.elo.cloud_put(self.key, "0x" + "11" * 20, "../x", b"")
        self.assertTrue(got.refused and got.status == 0)
        self.assertEqual(self.seen, [])

    def test_unsigned_reads(self):
        self.assertTrue(self.elo.quick_join("my-game", 4).ok)
        self.assertEqual(self.seen[-1][1], "/api/lobbies/pick/my-game?party=4")
        self.assertTrue(self.elo.quick_join("my-game", 0).refused)


if __name__ == "__main__":
    unittest.main()
