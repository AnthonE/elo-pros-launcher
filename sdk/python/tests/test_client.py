"""The HTTP client against a local stub server: a refusal, could-not-look,
and the guard that refuses to sign a text the change did not ask for."""
import json
import os
import socket
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from _testdata import fx

from elo_sdk import BadInput, Client, Mismatch, texts

os.environ["NO_PROXY"] = os.environ["no_proxy"] = "127.0.0.1,localhost"

ME = "0x1111111111111111111111111111111111111111"
THEM = "0x7777777777777777777777777777777777777777"
AT = 1761003600


class Stub:
    """A local server answering `route(path, body) -> (status, body)`, recording each path."""

    def __init__(self, route):
        self.seen = []
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def _answer(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n) or b"null")
                stub.seen.append(self.path)
                status, out = route(self.path, body)
                data = out if isinstance(out, bytes) else json.dumps(out).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = _answer

            def log_message(self, *_a):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.origin = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class FriendGuard(unittest.TestCase):
    def test_a_text_other_than_the_change_asked_for_is_never_signed(self):
        # Asked to ask THEM; the origin prepares a block instead.
        offered = texts.friends_sign_text(ME, "block", THEM, AT)
        stub = Stub(lambda path, body: (200, {"sign_this": offered, "at": AT, "args": {"wallet": THEM},
                                              "op": "block"}) if path == "/api/friends/prepare"
                    else (500, {}))
        signed = []
        try:
            with self.assertRaises(Mismatch) as caught:
                Client(stub.origin).friend(ME, "ask", THEM, lambda t: signed.append(t) or "0xsig")
        finally:
            stub.close()
        self.assertEqual(signed, [], "the signer was never asked")
        self.assertEqual(caught.exception.offered, offered)
        self.assertEqual(stub.seen, ["/api/friends/prepare"], "nothing was acted")

    def test_the_rebuilt_text_is_signed_once_and_acted(self):
        text = texts.friends_sign_text(ME, "ask", THEM, AT)

        def route(path, body):
            if path == "/api/friends/prepare":
                return 200, {"sign_this": text, "at": AT, "args": {"to": THEM}, "op": "ask"}
            if path == "/api/friends/act" and body["signature"] == "0xsig" and body["args"] == {"to": THEM}:
                return 200, {"ok": True, "op": "ask", "view": fx("friends_view.json")}
            return 401, {"error": "bad act"}

        stub = Stub(route)
        signed = []
        try:
            got = Client(stub.origin).friend(ME, "ask", THEM, lambda t: signed.append(t) or "0xsig")
        finally:
            stub.close()
        self.assertTrue(got.ok, got)
        self.assertEqual(len(got.value.view.buddies), 1)
        self.assertEqual(signed, [text])

    def test_bad_input_sends_nothing(self):
        c = Client("http://127.0.0.1:9")
        with self.assertRaises(BadInput):
            c.friend(ME, "poke", THEM, lambda t: "0x")
        with self.assertRaises(BadInput):
            c.friend(ME, "ask", ME, lambda t: "0x")
        self.assertEqual(c.friends("0x123").status, 0)
        self.assertTrue(c.leaderboard("gates", kind="robot").refused)


class Outcomes(unittest.TestCase):
    def test_a_refusal_and_a_dead_origin_are_different_answers(self):
        def route(path, body):
            if path.startswith("/api/friends/"):
                return 404, {"error": "not here"}
            if path.startswith("/api/ticket/gates/of/"):
                return 503, {"game": "gates", "entitled": None, "reachable": False, "why": "the chain read failed"}
            return 200, b"not json"

        stub = Stub(route)
        try:
            c = Client(stub.origin)
            got = c.friends(ME)
            self.assertTrue(got.refused)
            self.assertEqual((got.status, got.error), (404, "not here"))
            owns = c.owns("gates", ME)
            self.assertTrue(owns.could_not_look)
            self.assertEqual(owns.error, "the chain read failed")
            self.assertTrue(c.face(ME).could_not_look, "a body that is not JSON is could-not-look")
        finally:
            stub.close()
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        dead = f"http://127.0.0.1:{s.getsockname()[1]}"
        s.close()
        self.assertTrue(Client(dead, timeout=2).friends(ME).could_not_look, "nobody listening is could-not-look")


if __name__ == "__main__":
    unittest.main()
