"""The HTTP client against a local stub server: a refusal, could-not-look,
and the guard that refuses to sign a text the change did not ask for."""
import hashlib
import json
import os
import socket
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from _testdata import fx

from elo_sdk import BadInput, Client, Mismatch, play_message, texts

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
        self.assertTrue(c.guilds_of("0x123").refused)
        self.assertTrue(c.guild_tags([]).refused)
        self.assertTrue(c.guild_tags(["0x" + "1" * 40, "nope"]).refused)
        self.assertTrue(c.leaderboard("gates", kind="robot").refused)


def today() -> str:
    return texts.day_of(int(time.time()))


def buddy_text(wallet, op, args, at):
    detail = json.dumps({"args": args, "at": at, "op": op}, sort_keys=True, separators=(",", ":"))
    return play_message(texts.BUDDY_ACTION, wallet, detail, today())


class BuddyGuard(unittest.TestCase):
    GIFT = {"to": THEM, "usd": "5"}

    def run_buddy(self, offered_args, op="gift", asked=None, picture=None):
        offered = buddy_text(ME, op, offered_args, AT)
        acted = []

        def route(path, body):
            if path == "/api/buddy/prepare":
                return 200, {"sign_this": offered, "at": AT, "op": op, "args": offered_args}
            if path == "/api/buddy/act":
                acted.append(body)
                return 200, {"ok": True, "done": {"op": op}, "you": {}}
            return 404, {"error": "no"}

        stub = Stub(route)
        signed = []
        try:
            got = Client(stub.origin).buddy(ME, op, asked if asked is not None else self.GIFT,
                                            lambda t: signed.append(t) or "0xsig", picture_png=picture)
        finally:
            stub.close()
        return got, signed, acted, stub.seen

    def test_the_cleaned_change_is_signed_once_and_acted(self):
        got, signed, acted, _ = self.run_buddy({"to": THEM, "usd": 5.0})
        self.assertTrue(got.ok, got)
        self.assertEqual(signed, [buddy_text(ME, "gift", {"to": THEM, "usd": 5.0}, AT)])
        self.assertEqual(acted, [{"wallet": ME, "op": "gift", "args": self.GIFT, "at": AT, "signature": "0xsig"}])

    def test_a_gift_to_someone_else_is_never_signed(self):
        with self.assertRaises(Mismatch):
            self.run_buddy({"to": "0x" + "88" * 20, "usd": 5.0})
        with self.assertRaises(Mismatch):
            self.run_buddy({"to": THEM, "usd": 50.0})

    def test_an_argument_the_server_dropped_is_a_mismatch_and_nothing_is_signed(self):
        offered = buddy_text(ME, "gift", {"to": THEM, "usd": 5.0}, AT)
        stub = Stub(lambda path, body: (200, {"sign_this": offered, "at": AT}) if path == "/api/buddy/prepare"
                    else (500, {}))
        signed = []
        try:
            with self.assertRaises(Mismatch):
                Client(stub.origin).buddy(ME, "gift", {"to": THEM, "usd": 5, "memo": "hi"},
                                          lambda t: signed.append(t) or "0xsig")
        finally:
            stub.close()
        self.assertEqual(signed, [], "the signer was never asked")
        self.assertEqual(stub.seen, ["/api/buddy/prepare"], "nothing was acted")

    def test_a_picture_signs_its_sha(self):
        png = b"\x89PNG fake"
        sha = hashlib.sha256(png).hexdigest()
        got, _, acted, _ = self.run_buddy({"id": "a1", "pic_sha": sha}, op="face", asked={"id": "a1"}, picture=png)
        self.assertTrue(got.ok, got)
        self.assertEqual(acted[0]["args"], {"id": "a1", "pic_sha": sha})
        self.assertIn("picture_b64", acted[0])

    def test_bad_input_sends_nothing(self):
        c = Client("http://127.0.0.1:9")
        for op, args, pic in (("poke", {}, None), ("pause", [], None), ("pause", {"id": "a"}, b"png")):
            with self.subTest(op=op), self.assertRaises(BadInput):
                c.buddy(ME, op, args, lambda t: "0x", picture_png=pic)


class BoardGuard(unittest.TestCase):
    URL = "https://github.com/AnthonE/Gates/pull/412"

    def run_board(self, detail, op="submit"):
        offered = play_message(op, ME, detail, today())
        acted = []

        def route(path, body):
            if path == "/api/munus/gates-pr/prepare":
                return 200, {"op": op, "quest_id": "gates-pr", "detail": detail, "sign_this": offered}
            if path == f"/api/munus/gates-pr/{op}":
                acted.append(body)
                return 200, {"ok": True, "index": 2, "paid_only_if": "accepted"}
            return 404, {"error": "no"}

        stub = Stub(route)
        signed = []
        try:
            c = Client(stub.origin)
            got = (c.board_submit(ME, "gates-pr", self.URL, lambda t: signed.append(t) or "0xsig", note="hi")
                   if op == "submit" else c.board_claim(ME, "gates-pr", lambda t: signed.append(t) or "0xsig"))
        finally:
            stub.close()
        return got, signed, acted

    def test_a_delivery_signs_this_bounty_and_this_url(self):
        got, signed, acted = self.run_board(texts.submit_detail("gates-pr", self.URL, 2))
        self.assertTrue(got.ok, got)
        self.assertEqual(signed, [texts.submit_text(ME, "gates-pr", self.URL, 2, today())])
        self.assertEqual(acted, [{"wallet": ME, "signature": "0xsig", "url": self.URL, "note": "hi"}])

    def test_another_url_or_bounty_is_never_signed(self):
        for detail in (texts.submit_detail("gates-pr", self.URL + "1", 0),
                       texts.submit_detail("gates-soak", self.URL, 0), "gates-pr"):
            with self.subTest(detail=detail), self.assertRaises(Mismatch):
                self.run_board(detail)

    def test_a_claim_signs_the_bounty(self):
        got, signed, acted = self.run_board("gates-pr", op="claim")
        self.assertTrue(got.ok, got)
        self.assertEqual(signed, [texts.claim_text(ME, "gates-pr", today())])
        with self.assertRaises(Mismatch):
            self.run_board("gates-soak", op="claim")


class EntitleGuard(unittest.TestCase):
    def test_the_rebuilt_text_is_signed_and_the_grant_comes_back(self):
        sent = []

        def route(path, body):
            if path == f"/api/ticket/gates/message?wallet={ME}":
                return 200, {"message": texts.entitle_text("gates", ME, today())}
            if path == "/api/ticket/gates/entitle":
                sent.append(body)
                return 200, {"grant": "g.1", "expires_at": 1}
            return 404, {"error": "no"}

        stub = Stub(route)
        try:
            got = Client(stub.origin).entitle("gates", ME.upper().replace("0X", "0x"), lambda t: "0xsig")
        finally:
            stub.close()
        self.assertEqual(got.value["grant"], "g.1")
        self.assertEqual(sent, [{"wallet": ME, "signature": "0xsig"}])

    def test_another_wallets_text_is_never_signed(self):
        stub = Stub(lambda path, body: (200, {"message": texts.entitle_text("gates", THEM, today())}))
        try:
            with self.assertRaises(Mismatch):
                Client(stub.origin).entitle("gates", ME, lambda t: self.fail("signed"))
        finally:
            stub.close()


class Reads(unittest.TestCase):
    def test_each_read_asks_its_path(self):
        stub = Stub(lambda path, body: (200, {"ok": True}))
        try:
            c = Client(stub.origin)
            for got in (c.home(ME, since=5), c.home(), c.floor(), c.buddy_offer(), c.buddies(ME), c.board(),
                        c.ticket_buy("gates", wallet=ME, rail="free")):
                self.assertTrue(got.ok, got)
        finally:
            stub.close()
        self.assertEqual(stub.seen, [f"/api/home?wallet={ME}&since=5", "/api/home", "/api/agents/floor",
                                     "/api/buddy", f"/api/buddy/{ME}", "/api/munus?game=gates",
                                     f"/api/ticket/gates/prepare?rail=free&wallet={ME}"])
        c = Client("http://127.0.0.1:9")
        self.assertEqual(c.home("0x12").status, 0)
        self.assertEqual(c.home(ME, since=-1).status, 0)
        self.assertEqual(c.buddies("x").status, 0)
        self.assertEqual(c.board("../x").status, 0)


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
