"""A game server's half: the sign-in check, the roster sweep and the nonce,
against a local stub server that answers the way the meter does."""
import json
import os
import re
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from _testdata import fx

from elo_sdk import ROSTER_PAGE, Client, new_nonce

os.environ["NO_PROXY"] = os.environ["no_proxy"] = "127.0.0.1,localhost"

SIGNIN = fx("signin_check.json")
EVM, SOL = SIGNIN["cases"][0], SIGNIN["cases"][1]
SOLW = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB"


def W(i: int) -> str:
    return "0x" + format(i, "040x")


class Stub:
    """A local server answering `route(path, body) -> (status, body)`, recording each request."""

    def __init__(self, route):
        self.seen = []
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def _answer(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n) or b"null")
                stub.seen.append((self.command, self.path, body))
                status, out = route(self.path, body)
                data = json.dumps(out).encode()
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


class CheckSignIn(unittest.TestCase):
    def test_sends_message_signature_domain_and_nonce(self):
        stub = Stub(lambda path, body: (200, {"ok": True, "address": EVM["expect"]["address"], "chain": "evm",
                                              "chain_id": "4663", "domain": body["domain"], "nonce": body["nonce"],
                                              "uri": "https://game.example", "issued_at": "x"})
                    if path == "/api/signin/check" else (404, {"error": "no"}))
        try:
            got = Client(stub.origin).check_sign_in(EVM["message"], EVM["signature"], domain=EVM["domain"],
                                                    nonce=EVM["nonce"], chain_id=4663, max_age_s=300)
        finally:
            stub.close()
        self.assertTrue(got.ok, got)
        self.assertEqual(got.value.address, EVM["expect"]["address"])
        self.assertEqual(got.value.chain, "evm")
        method, path, body = stub.seen[0]
        self.assertEqual((method, path), ("POST", "/api/signin/check"))
        self.assertEqual(body, {"message": EVM["message"], "signature": EVM["signature"], "domain": EVM["domain"],
                                "nonce": EVM["nonce"], "chain_id": "4663", "max_age_s": 300})

    def test_a_failed_check_is_refused_with_the_field(self):
        stub = Stub(lambda path, body: (401, {"ok": False, "code": "domain", "error": "this sign-in is for x"}))
        try:
            got = Client(stub.origin).check_sign_in(SOL["message"], SOL["signature"], domain="other.example",
                                                    nonce=SOL["nonce"])
        finally:
            stub.close()
        self.assertTrue(got.refused)
        self.assertEqual(got.status, 401)
        self.assertEqual(got.body["code"], "domain")

    def test_an_unreachable_chain_is_could_not_look(self):
        stub = Stub(lambda path, body: (503, {"ok": False, "reachable": False, "code": "unreachable",
                                              "error": "could not reach the chain"}))
        try:
            got = Client(stub.origin).check_sign_in(EVM["message"], EVM["signature"], domain=EVM["domain"],
                                                    nonce=EVM["nonce"])
        finally:
            stub.close()
        self.assertTrue(got.could_not_look)
        self.assertIn("could not reach the chain", got.error)

    def test_a_200_that_is_not_a_checked_sign_in_is_could_not_look(self):
        stub = Stub(lambda path, body: (200, {"ok": True}))
        try:
            got = Client(stub.origin).check_sign_in(EVM["message"], EVM["signature"], domain=EVM["domain"],
                                                    nonce=EVM["nonce"])
        finally:
            stub.close()
        self.assertTrue(got.could_not_look, got)

    def test_refused_before_asking_without_a_domain_or_nonce(self):
        stub = Stub(lambda path, body: (200, {"ok": True}))
        c = Client(stub.origin)
        try:
            for kw in ({"domain": "", "nonce": EVM["nonce"]}, {"domain": EVM["domain"], "nonce": ""},
                       {"domain": "https://game.example", "nonce": EVM["nonce"]},
                       {"domain": "game.example/play", "nonce": EVM["nonce"]},
                       {"domain": EVM["domain"], "nonce": "550e8400-e29b-41d4"},
                       {"domain": EVM["domain"], "nonce": EVM["nonce"], "max_age_s": 0},
                       {"domain": EVM["domain"], "nonce": EVM["nonce"], "max_age_s": True}):
                got = c.check_sign_in(EVM["message"], EVM["signature"], **kw)
                self.assertTrue(got.refused and got.status == 0, kw)
            self.assertEqual(c.check_sign_in("", "0x", domain=EVM["domain"], nonce=EVM["nonce"]).status, 0)
        finally:
            stub.close()
        self.assertEqual(stub.seen, [], "nothing was sent")


class CheckRoster(unittest.TestCase):
    def test_pages_the_roster_and_keys_every_wallet(self):
        roster = [W(i + 1) for i in range(ROSTER_PAGE + 5)] + [W(1).upper().replace("0X", "0x"), SOLW]
        stub = Stub(lambda path, body: (200, {"game": "gates",
                                              "wallets": {w: {"entitled": w != W(3)} for w in body["wallets"]}}))
        try:
            got = Client(stub.origin).check_roster("gates", roster)
        finally:
            stub.close()
        self.assertTrue(got.ok, got)
        self.assertEqual(len(stub.seen), 2)
        self.assertEqual(len(stub.seen[0][2]["wallets"]), ROSTER_PAGE)
        self.assertTrue(all(m == "POST" and p == "/api/ticket/gates/check" for m, p, _ in stub.seen))
        self.assertEqual(len(got.value.entitled), ROSTER_PAGE + 6, "a repeated wallet is swept once")
        self.assertIs(got.value.entitled[W(3)], False)
        self.assertIs(got.value.entitled[W(4)], True)
        self.assertIs(got.value.entitled[SOLW], True)
        self.assertTrue(got.value.ticketed)

    def test_a_page_that_could_not_be_read_is_none_never_a_kick(self):
        calls = []

        def route(path, body):
            calls.append(1)
            if len(calls) == 2:
                return 502, {"error": "rpc down"}
            return 200, {"game": "gates", "wallets": {w: {"entitled": False} for w in body["wallets"]}}
        stub = Stub(route)
        try:
            got = Client(stub.origin).check_roster("gates", [W(i + 1) for i in range(ROSTER_PAGE + 2)])
        finally:
            stub.close()
        self.assertTrue(got.ok)
        self.assertIs(got.value.entitled[W(1)], False)
        self.assertIsNone(got.value.entitled[W(ROSTER_PAGE + 1)])

    def test_a_row_left_out_or_unread_is_none(self):
        stub = Stub(lambda path, body: (200, {"game": "gates", "wallets": {W(1): {"entitled": None}}}))
        try:
            got = Client(stub.origin).check_roster("gates", [W(1), W(2)])
        finally:
            stub.close()
        self.assertEqual(got.value.entitled, {W(1): None, W(2): None})

    def test_no_page_read_is_could_not_look_and_a_free_title_is_everyones(self):
        stub = Stub(lambda path, body: (503, {"error": "down"}))
        try:
            self.assertTrue(Client(stub.origin).check_roster("gates", [W(1)]).could_not_look)
        finally:
            stub.close()
        stub = Stub(lambda path, body: (200, {"game": "free-game", "ticketed": False,
                                              "every_wallet_entitled": True}))
        try:
            got = Client(stub.origin).check_roster("free-game", [W(1), SOLW])
        finally:
            stub.close()
        self.assertFalse(got.value.ticketed)
        self.assertEqual(got.value.entitled, {W(1): True, SOLW: True})

    def test_a_bad_roster_is_refused_before_asking(self):
        stub = Stub(lambda path, body: (200, {}))
        c = Client(stub.origin)
        try:
            for slug, wallets in (("gates", []), ("gates", W(1)), ("gates", [W(1), "0xnope"]),
                                  ("Not A Slug", [W(1)])):
                got = c.check_roster(slug, wallets)
                self.assertTrue(got.refused and got.status == 0, (slug, wallets))
        finally:
            stub.close()
        self.assertEqual(stub.seen, [])

    def test_owns_takes_a_solana_wallet_as_given(self):
        stub = Stub(lambda path, body: (200, {"game": "gates", "wallet": SOLW, "entitled": False}))
        try:
            got = Client(stub.origin).owns("gates", SOLW)
        finally:
            stub.close()
        self.assertTrue(got.ok)
        self.assertIs(got.value, False)
        self.assertEqual(stub.seen[0][1], f"/api/ticket/gates/of/{SOLW}")


class NewNonce(unittest.TestCase):
    def test_alphabet_length_and_fresh(self):
        seen = {new_nonce() for _ in range(200)}
        self.assertEqual(len(seen), 200)
        self.assertTrue(all(re.fullmatch(r"[A-Za-z0-9]{16}", n) for n in seen))
        self.assertRegex(new_nonce(8), r"^[A-Za-z0-9]{8}$")
        for bad in (7, 129, True, "16"):
            with self.assertRaises(ValueError):
                new_nonce(bad)


if __name__ == "__main__":
    unittest.main()
